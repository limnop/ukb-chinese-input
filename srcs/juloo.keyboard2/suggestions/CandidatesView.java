package juloo.keyboard2.suggestions;

import android.content.Context;
import android.graphics.Typeface;
import android.os.Build.VERSION;
import android.text.InputType;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.TextPaint;
import android.text.style.CharacterStyle;
import android.text.style.RelativeSizeSpan;
import android.text.style.StyleSpan;
import android.util.AttributeSet;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.inputmethod.EditorInfo;
import android.widget.AbsListView;
import android.widget.BaseAdapter;
import android.widget.GridView;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import juloo.keyboard2.Config;
import juloo.keyboard2.KeyboardData;
import juloo.keyboard2.R;
import juloo.keyboard2.VibratorCompat;
import juloo.keyboard2.staged.Composer;

public class CandidatesView extends LinearLayout
  implements CandidatesScrollView.OnScroll
{
  /** Number of cells added at a time. The next ones are added when the row is
      scrolled near its end, so that a long list of candidates is shown without
      creating every view. */
  static final int CELLS_PER_PAGE = 4;

  /** The candidates to show. The emoji suggestion, when there is one, is the
      last entry and has no annotation. */
  List<Composer.Candidate> _candidates = Collections.emptyList();
  /** Number of entries of the current page that have a cell. */
  int _shown = 0;
  /** Index in [_candidates] of the first entry of the current page. The
      up/down arrows move it; the digit keys select relative to it, so '1'
      always picks the leftmost word that is actually on screen. */
  int _page_start = 0;

  /** The width available to the row, or 0 before the first layout. */
  int _width = 0;

  CandidatesScrollView _scroll;
  LinearLayout _row;

  /** Toggle between the one-row strip and a grid of every candidate. The grid
      and the keyboard are siblings in [R.layout.keyboard], looked up lazily. */
  ImageView _expand_button;
  /** Small permanent mark, shown only on Chinese layouts. */
  TextView _mode_indicator;
  /** The latin letters being composed, a sibling of this view in the keyboard
      layout. */
  TextView _composing_view;
  GridView _grid;
  View _keyboard_view;
  GridAdapter _grid_adapter;
  boolean _expanded = false;

  /** Label size of a candidate and of its annotation, or 0 before
      [set_sizes]. */
  float _text_size = 0;
  float _annotation_size = 0;
  /** Fixed, larger size of a candidate's text. Auto-sizing used to shrink a
      long word until it was unreadable. */
  float _candidate_size = 0;

  public CandidatesView(Context context, AttributeSet attrs)
  {
    super(context, attrs);
  }

  @Override
  protected void onFinishInflate()
  {
    super.onFinishInflate();
    _scroll = (CandidatesScrollView)findViewById(R.id.candidates_scroll);
    _row = (LinearLayout)findViewById(R.id.candidates_row);
    _scroll.set_on_scroll(this);
    _mode_indicator = (TextView)findViewById(R.id.candidates_mode);
    _expand_button = (ImageView)findViewById(R.id.candidates_expand);
    _expand_button.setOnClickListener(new View.OnClickListener()
        {
          @Override
          public void onClick(View _v)
          {
            toggle_expanded();
          }
        });
  }

  @Override
  protected void onSizeChanged(int w, int h, int oldw, int oldh)
  {
    super.onSizeChanged(w, h, oldw, oldh);
    if (w != oldw)
    {
      _width = w;
      layout_cells();
    }
  }

  /** Lay out the visible part of the row. Cells share the width equally,
      never narrower than [candidates_min_width], except that a cell holding a
      long word grows to fit it and the row is scrolled instead. Only the cells
      needed to fill the screen are built; [on_scroll] adds the rest. */
  void layout_cells()
  {
    int n = page_size();
    if (n == 0 || _width == 0)
      return;
    int avail = _width - getPaddingEnd();
    int gap = getResources().getDimensionPixelSize(R.dimen.candidates_gap);
    int min = getResources().getDimensionPixelSize(R.dimen.candidates_min_width);
    /* The pitch is the share of one of the cells that fit, not of every
       candidate: a list longer than the row would otherwise take the minimum
       width for every cell and leave the row short of its end. */
    int slot = Math.max(min, avail / Math.min(n, Math.max(1, avail / min)));
    /* Only build the cells that can be seen; [on_scroll] adds the rest when
       the row nears its end. Prefix candidates can number in the hundreds and
       inflating all of them on every keystroke made typing stall. */
    int visible = Math.min(n, Math.max(_shown, Math.max(1, avail / min) + 1));
    if (visible > _shown)
      add_cells(visible - _shown);
    int width = Math.max(0, slot - gap);
    for (int i = 0; i < _row.getChildCount(); i++)
    {
      View cell = _row.getChildAt(i);
      /* A word or phrase longer than one character must not be squeezed into
         the shared slot: give that cell the width its text needs and let the
         row scroll instead. */
      int w = width;
      TextView text = (TextView)cell.findViewById(R.id.candidate_text);
      if (_candidate_size > 0 && text != null)
        w = Math.max(w, (int)(text.getText().length() * _candidate_size * 1.1f)
            + 2 * gap);
      ViewGroup.LayoutParams lp = cell.getLayoutParams();
      if (lp.width != w)
      {
        lp.width = w;
        cell.setLayoutParams(lp);
      }
    }
  }

  public void set_candidates(Suggestions s)
  {
    List<Composer.Candidate> cs = new ArrayList<Composer.Candidate>();
    cs.addAll(s.candidates);
    if (s.emoji_suggestion != null)
      cs.add(new Composer.Candidate(s.emoji_suggestion));
    _candidates = cs;
    _page_start = 0;
    _row.removeAllViews();
    _shown = 0;
    add_cells(CELLS_PER_PAGE);
    layout_cells();
    _scroll.scrollTo(0, 0);
    if (_expanded)
      refresh_grid();
    set_composing(s.composing, s.composing_cursor);
    update_controls();
  }

  /** The one character the control shows and the reading that goes above it, in
      that order, or [null] for a language whose candidates never carry a
      reading.

      One fixed exemplar per language rather than a candidate of the current
      row. A sample of the row changes on every keystroke, so the control could
      never become a symbol the user learns; for Japanese the reading belongs
      to the word, so a sample drawn from a multi-character candidate pairs a
      character with a reading it does not have; and sampling also made the
      control disappear whenever the row happened to hold no reading, which
      loses the switch mid-typing.

      Japanese draws the character whose reading is the language's own name for
      a reading written above a character - a true pair, and it says what the
      control shows. Chinese draws the character for writing, with its rising
      second tone above it.

      The two are deliberately not the same glyph: no user ever sees both, since
      the control is per language, so parallelism between them buys nothing and
      each glyph earns its place on its own grounds.

      The Chinese one was chosen against two criteria, in this order. The control
      exists to show a reading, so the mark it carries has to be legible at the
      control's reading size - three quarters of the control's size, which is the
      annotation size - and the flat first tone is the faintest of the four
      marks, so a character that takes it demonstrates the worst case rather than
      the best: that is what ruled out the character for China. And the glyph has
      to be the same in simplified and traditional, because the build ships
      zh_CN, zh_SG and zh_TW: that is what ruled out the character for language,
      which was otherwise the most apt, as it would have needed two glyphs picked
      by subtype. This one takes the rising tone, which is unmistakable, and is
      script-invariant. Anything else, Korean included, has no reading to show. */
  static String[] rubytext_exemplar()
  {
    switch (current_script())
    {
      case "kana":
        return new String[]{ "\u97f3", "\u304a\u3093" }; /* sound, "on" */
      case "pinyin":
      case "zhuyin":
        return new String[]{ "\u6587", "\u02ca" }; /* writing, rising tone */
      default:
        return null;
    }
  }

  /** The script the layout in front of the user declares - "kana", "pinyin",
      "zhuyin", "hangul" - or "" when there is none.

      Not the app's language. [DeviceLocales.OFFERED] is a table the build pins,
      so the language reads "ja_JP" on every layout; a script comes from the
      layout file, and changing layout changes it. Nothing pins a layout file.

      Read from the layout, not from the composer, so an idle row - no
      composition, only dictionary suggestions - still knows what it is showing
      and keeps the control. */
  static String current_script()
  {
    Config config = Config.globalConfig();
    int i = config.get_current_layout();
    if (i < 0 || i >= config.layouts.size())
      return "";
    KeyboardData layout = config.layouts.get(i);
    return layout == null ? "" : layout.script;
  }

  /** Whether the layout in front of the user annotates with a tone mark. An
      empty script - the system layout - is not a language that annotates. */
  static boolean is_chinese()
  {
    String script = current_script();
    return script.equals("pinyin") || script.equals("zhuyin");
  }

  /** How much larger than its line a Chinese tone mark is drawn. The marks are
      spacing modifier letters - U+02C9, U+02CA, U+02C7 and U+02CB - whose ink
      is a thin stroke high in the line: measured in the pinyin row it was 5px
      against a main text of 30px; the Chinese session measured that the hinted ink plateaus below 2x. 2.2x puts it at 10px, measured on the device. */
  static final float MARK_SCALE = 2.2f;
  /* Measured, not extrapolated: the mark is a thin stroke and hinting quantises
     its growth away. U+02CA in this box: 13.6px em -> 2px ink, 23.1px (1.7x) -> 4px,
     27.2px (2.0x) -> 4px, 40px -> 6px. Bold thickens the stroke without raising the
     ink. 2.2x is the chosen scale; 2.0x is the floor. The rows still show what the
     capture measures. */

  /** Draws a mark larger and in a heavier stroke than its line is laid out
      with. Deliberately a drawing-only span rather than a relative size span:
      a span that changed the metrics would make every cell taller and move the
      text inside it, which is the alignment trap of 44c396a. */
  static class HeavierMark extends CharacterStyle
  {
    float _scale;

    HeavierMark(float scale)
    {
      _scale = scale;
    }

    @Override
    public void updateDrawState(TextPaint tp)
    {
      tp.setTextSize(tp.getTextSize() * _scale);
      tp.setFakeBoldText(true);
    }
  }

  /** The annotation as it is drawn. Japanese readings are kana and are read
      well enough at the annotation size; Chinese tone marks are not, so they
      are drawn bigger and bolder. */
  CharSequence shown_annotation(String annotation)
  {
    if (annotation == null || !is_chinese())
      return annotation;
    SpannableString s = new SpannableString(annotation);
    s.setSpan(new HeavierMark(MARK_SCALE), 0, annotation.length(),
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
    return s;
  }

  /** Whether the annotation of a candidate cell is drawn. When the readings
      are on, the line is kept even for a candidate that has no reading so
      that the cells stay aligned; when they are off, the line goes so that
      the row does not keep a blank one. */
  int annotation_visibility()
  {
    /* Text-only candidates: the annotation line is always gone. */
    return View.GONE;
  }

  /** Add up to [count] more candidate cells. */
  void add_cells(int count)
  {
    int end = Math.min(_shown + count, page_size());
    if (end <= _shown)
      return;
    LayoutInflater inflater = LayoutInflater.from(getContext());
    while (_shown < end)
    {
      final Composer.Candidate candidate = _candidates.get(_page_start + _shown);
      View cell = inflater.inflate(R.layout.candidates_item, _row, false);
      TextView text = (TextView)cell.findViewById(R.id.candidate_text);
      text.setText(candidate.text);
      /* Annotations are never drawn, so their content is not built at all:
         no SpannableString, no per-cell layout parameters. */
      cell.findViewById(R.id.candidate_annotation).setVisibility(View.GONE);
      /* One fixed size, no auto-sizing: a long word must not shrink. */
      set_text_size(text, _candidate_size, false);
      cell.setOnClickListener(new View.OnClickListener()
          {
            @Override
            public void onClick(View _v)
            {
              VibratorCompat.vibrate(_v, Config.globalConfig());
              Config.globalConfig().handler.suggestion_entered(candidate);
            }
          });
      _row.addView(cell);
      _shown++;
    }
  }

  /** Called when the row is scrolled: add the next cells near the end. */
  @Override
  public void on_scroll(int scrollX, int scrollY)
  {
    if (_shown < page_size()
        && scrollX + _scroll.getWidth() + _scroll.getWidth() / 2
           >= _row.getWidth())
    {
      add_cells(CELLS_PER_PAGE);
      layout_cells();
    }
  }

  /** How many candidates fit on one page. Independent of the current page,
      so paging always advances by the same number of words. */
  int per_page()
  {
    int min = getResources().getDimensionPixelSize(R.dimen.candidates_min_width);
    return Math.max(CELLS_PER_PAGE, _width / Math.max(1, min));
  }

  /** Number of candidates on the current page. */
  int page_size()
  {
    return Math.min(per_page(), Math.max(0, _candidates.size() - _page_start));
  }

  /** Candidate at [digit] on the page that is actually shown, or [null] past
      its end. The digit keys ask the row itself, so the index can never drift
      from what the user sees after paging. */
  public Composer.Candidate candidate_at_digit(int digit)
  {
    int i = _page_start + digit;
    return (i >= 0 && i < _candidates.size()) ? _candidates.get(i) : null;
  }

  /** Rebuild the row for the current page, back at the left edge. Only the
      cells of one page exist at a time. */
  void rebuild_page()
  {
    _row.removeAllViews();
    _shown = 0;
    add_cells(page_size());
    layout_cells();
    if (_scroll != null)
      _scroll.scrollTo(0, 0);
    update_controls();
  }

  /** Page through the candidates with the up/down arrows. [dir] is -1 toward
      the first candidates and +1 toward the later ones. At either end the
      row simply stays where it is. */
  public boolean page_candidates(int dir)
  {
    if (_candidates.isEmpty() || _scroll == null)
      return false;
    int next = _page_start + dir * per_page();
    if (next < 0 || next >= _candidates.size())
      return false;
    _page_start = next;
    rebuild_page();
    return true;
  }

  void clear_candidates()
  {
    _candidates = Collections.emptyList();
    _page_start = 0;
    _shown = 0;
    _row.removeAllViews();
    if (_expanded)
      refresh_grid();
    set_composing("", 0);
    update_controls();
  }

  public void refresh_config(Config config)
  {
    collapse();
    clear_candidates();
    set_sizes(config);
  }

  /** Set the height of the suggestion row and the text size. */
  void set_sizes(Config config)
  {
    // Make the candidates view about as high as a keyboard row.
    float row_height = config.keyboard_rows_height_pixels * (1 - config.key_vertical_margin);
    ViewGroup.MarginLayoutParams p =
        (ViewGroup.MarginLayoutParams)getLayoutParams();
    /* Shorter than a keyboard row. The label sizes below are still derived
       from the full row height: shrinking the bar must not shrink the
       candidates with it. */
    p.height = (int)(row_height * 0.72f);
    setLayoutParams(p);
    // Match the size of labels on the keyboard.
    _text_size = row_height * config.characterSize * config.labelTextSize;
    _annotation_size = _text_size * 0.4f;
    /* Candidates are what the user reads: larger than the key labels, and
       fixed. Auto-sizing used to shrink a long word until it was unreadable. */
    _candidate_size = _text_size * 1.15f;
    update_mode_indicator();
    for (int i = 0; i < _row.getChildCount(); i++)
    {
      View cell = _row.getChildAt(i);
      set_text_size((TextView)cell.findViewById(R.id.candidate_text),
          _candidate_size, false);
    }
  }

  /** Set the text size of a candidate or of its annotation. Auto sizing is
      only used for the candidate itself. */
  void set_text_size(TextView v, float size, boolean autosize)
  {
    if (size <= 0)
      return;
    if (VERSION.SDK_INT < 26 || !autosize)
      v.setTextSize(TypedValue.COMPLEX_UNIT_PX, size);
    else
      v.setAutoSizeTextTypeUniformWithConfiguration(
          (int)(size / 2.), (int)size, 1, TypedValue.COMPLEX_UNIT_PX);
  }

  /* The strip deliberately has no dictionary-install hint, language name or
     language-switch button: without a dictionary those three occupied the
     entire bar. Only the candidates and the expand control are shown. */

  /** The grid and the keyboard are siblings in [R.layout.keyboard]. They are
      looked up lazily: [onFinishInflate] can run before this view is attached
      to its parent. */
  void ensure_siblings()
  {
    ViewParent parent = getParent();
    if (!(parent instanceof View))
      return;
    View container = (View)parent;
    if (_composing_view == null)
      _composing_view =
        (TextView)container.findViewById(R.id.candidates_composing);
    if (_grid != null && _keyboard_view != null)
      return;
    _grid = (GridView)container.findViewById(R.id.candidates_grid);
    _keyboard_view = container.findViewById(R.id.keyboard_view);
    if (_grid != null && _grid_adapter == null)
    {
      _grid_adapter = new GridAdapter();
      _grid.setAdapter(_grid_adapter);
    }
  }

  /** Swap the keyboard area for a scrollable grid holding every candidate.
      The grid takes the exact height of the keyboard, so the input view keeps
      its size and the app below is not resized. Tapping again restores it. */
  void toggle_expanded()
  {
    ensure_siblings();
    if (_grid == null || _keyboard_view == null)
      return;
    _expanded = !_expanded;
    if (_expanded)
    {
      int h = _keyboard_view.getHeight();
      if (h <= 0)
        h = _keyboard_view.getMeasuredHeight();
      if (h <= 0)
        h = (int)(getResources().getDisplayMetrics().density * 240);
      ViewGroup.LayoutParams lp = _grid.getLayoutParams();
      lp.height = h;
      _grid.setLayoutParams(lp);
      _keyboard_view.setVisibility(View.GONE);
      _grid.setVisibility(View.VISIBLE);
    }
    else
    {
      _grid.setVisibility(View.GONE);
      _keyboard_view.setVisibility(View.VISIBLE);
    }
    update_expand_icon();
    refresh_grid();
    requestLayout();
  }

  /** Return to the keyboard, e.g. when the editor or the configuration
      changes. Safe to call while already collapsed. */
  void collapse()
  {
    if (!_expanded)
      return;
    _expanded = false;
    ensure_siblings();
    if (_grid != null)
      _grid.setVisibility(View.GONE);
    if (_keyboard_view != null)
      _keyboard_view.setVisibility(View.VISIBLE);
    update_expand_icon();
    requestLayout();
  }

  /** Show the pinyin letters being composed above the row. Chinese layouts
      only: on a latin layout the editor already shows exactly what was
      typed. The line is a sibling kept GONE when unused, so the keyboard
      does not grow while typing English. */
  void set_composing(String text, int cursor)
  {
    ensure_siblings();
    if (_composing_view == null)
      return;
    boolean show = is_chinese() && text.length() > 0
        && getVisibility() == View.VISIBLE;
    if (show)
    {
      int c = Math.max(0, Math.min(cursor, text.length()));
      /* The caret is a literal bar inside the line: no custom drawing, no
         extra view, and it moves with the arrow keys exactly like the text
         it belongs to. */
      SpannableString s = new SpannableString(
          text.substring(0, c) + "|" + text.substring(c));
      s.setSpan(new StyleSpan(Typeface.BOLD), c, c + 1,
          Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
      _composing_view.setText(s);
    }
    else
      _composing_view.setText("");
    _composing_view.setVisibility(show ? View.VISIBLE : View.GONE);
  }

  /** Keep the two controls in sync with the row. The expand button is
      pointless with no candidates, and an empty row must also close the grid:
      otherwise the button disappears and there is nothing left to close it
      with. The mode mark follows the layout's script, not the row. */
  void update_controls()
  {
    boolean has_candidates = !_candidates.isEmpty();
    if (!has_candidates && _expanded)
      collapse();
    if (_expand_button != null)
      _expand_button.setVisibility(has_candidates ? View.VISIBLE : View.GONE);
    update_mode_indicator();
  }

  /** A tiny mark pinned to the top-left corner: 中 on a Chinese layout, Eng
      otherwise. Always present, so the mode stays readable even when there
      are no candidates. English is drawn smaller because three letters take
      more room than one glyph. */
  void update_mode_indicator()
  {
    if (_mode_indicator == null)
      return;
    boolean chinese = is_chinese();
    _mode_indicator.setText(chinese ? "中" : "Eng");
    _mode_indicator.setContentDescription(
        chinese ? "中文输入" : "English input");
    if (_text_size > 0)
      set_text_size(_mode_indicator,
          _text_size * (chinese ? 0.42f : 0.32f), false);
  }

  /** The control is an icon, and the icon states which way it will toggle. */
  void update_expand_icon()
  {
    if (_expand_button == null)
      return;
    _expand_button.setImageResource(_expanded ?
        R.drawable.ic_candidates_collapse : R.drawable.ic_candidates_expand);
    _expand_button.setContentDescription(
        _expanded ? "收起候选字表" : "展开候选字表");
  }

  void refresh_grid()
  {
    ensure_siblings();
    if (_grid_adapter == null)
      return;
    /* Widen the columns to fit the longest candidate: a two-character word
       or a short phrase must not be clipped in the expanded table. */
    int longest = 1;
    for (Composer.Candidate c : _candidates)
      longest = Math.max(longest, c.text.length());
    float density = getResources().getDisplayMetrics().density;
    int min_w = (int)(38 * density);
    int w = min_w;
    if (_candidate_size > 0)
      w = Math.max(w, (int)(longest * _candidate_size * 1.02f)
          + (int)(8 * density));
    _grid.setColumnWidth(w);
    _grid_adapter.notifyDataSetChanged();
  }

  /** One cell per candidate, text only, sharing the strip's label colour and
      cell background. A tap commits the word; the grid stays open while there
      are still candidates, and closes itself when the row is emptied. */
  class GridAdapter extends BaseAdapter
  {
    @Override
    public int getCount()
    {
      return _expanded ? _candidates.size() : 0;
    }

    @Override
    public Object getItem(int i)
    {
      return _candidates.get(i).text;
    }

    @Override
    public long getItemId(int i)
    {
      return i;
    }

    @Override
    public View getView(int i, View reused, ViewGroup parent)
    {
      TextView cell;
      if (reused instanceof TextView)
        cell = (TextView)reused;
      else
      {
        cell = new TextView(getContext(), null, 0, R.style.candidates_text);
        cell.setGravity(Gravity.CENTER);
        cell.setBackgroundResource(R.drawable.suggestions_item_background);
        int pad = (int)(getResources().getDisplayMetrics().density * 2);
        cell.setPadding(pad, pad, pad, pad);
        int cell_h = (int)(_candidate_size * 1.65f);
        if (cell_h < 48)
          cell_h = 48;
        cell.setLayoutParams(new AbsListView.LayoutParams(
              ViewGroup.LayoutParams.MATCH_PARENT, cell_h));
      }
      final Composer.Candidate candidate = _candidates.get(i);
      cell.setText(candidate.text);
      if (_candidate_size > 0)
        cell.setTextSize(TypedValue.COMPLEX_UNIT_PX, _candidate_size);
      cell.setOnClickListener(new View.OnClickListener()
          {
            @Override
            public void onClick(View _v)
            {
              VibratorCompat.vibrate(_v, Config.globalConfig());
              Config.globalConfig().handler.suggestion_entered(candidate);
            }
          });
      return cell;
    }
  }

  /** Whether the candidates view should be shown for a given editor. */
  public static boolean should_show(EditorInfo info)
  {
    int variation = info.inputType & InputType.TYPE_MASK_VARIATION;
    int flags = info.inputType & InputType.TYPE_MASK_FLAGS;
    switch (info.inputType & InputType.TYPE_MASK_CLASS)
    {
      case InputType.TYPE_CLASS_TEXT:
        switch (variation)
        {
          case InputType.TYPE_TEXT_VARIATION_PASSWORD:
          case InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD:
          case InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD:
            return false;
          default:
            /* Editor requested that we don't show suggestions. Enable
               suggestions anyway when the flags [NO_SUGGESTIONS] and
               [AUTO_CORRECT] are present at the same time. This happens with
               Google Keep. */
            if ((flags &
                  (InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                   | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT))
                == InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS)
              return false;
            return true;
        }
      case InputType.TYPE_CLASS_NUMBER:
        // Beware of TYPE_NUMBER_VARIATION_PASSWORD
        return false;
      default: return false;
    }
  }
}
