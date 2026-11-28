package juloo.keyboard2;

import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.Typeface;
import android.graphics.drawable.BitmapDrawable;
import android.graphics.drawable.Drawable;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.style.ImageSpan;
import android.util.AttributeSet;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
import android.widget.AbsListView;
import android.widget.BaseAdapter;
import android.widget.GridView;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

public class EmojiGridView extends GridView
{
  private static final String LAST_USE_PREF = "emoji_last_use";

  private final Map<Emoji, Integer> _lastUsed = new HashMap<>();
  private final Paint _physicalHighlight = new Paint(Paint.ANTI_ALIAS_FLAG);
  private final Drawable _paneBackground;
  private Emoji.Group _group;
  private Emoji.Group _unicodeGroup;
  private NativeEmoji _platform;
  private boolean _choosingVariants;
  private int _categoryPosition;
  private Runnable _physicalSelectionCallback;
  private int _physicalPosition;

  public EmojiGridView(Context context, AttributeSet attrs)
  {
    super(context, attrs);
    _paneBackground = getBackground();
    Emoji.init(context.getResources());
    TypedValue color = new TypedValue();
    _physicalHighlight.setColor(context.getTheme().resolveAttribute(
        R.attr.colorLabelActivated, color, true) ? color.data : 0xff3399ff);
    _physicalHighlight.setStyle(Paint.Style.STROKE);
    _physicalHighlight.setStrokeWidth(2 * getResources().getDisplayMetrics().density);
    loadLastUsed();
    setEmojiGroup(_lastUsed.isEmpty() ? Emoji.Group.FACES : Emoji.Group.RECENT);
    setOnItemClickListener((parent, view, position, id) -> view.performClick());
    setOnItemLongClickListener((parent, view, position, id) -> view.performLongClick());
  }

  @Override
  protected void onAttachedToWindow()
  {
    super.onAttachedToWindow();
    updateGroupBar();
  }

  void setEmojiGroup(Emoji.Group group)
  {
    if (group == Emoji.Group.NATIVE && _platform == null)
      return;
    _group = group;
    _choosingVariants = false;
    List<Emoji> entries;
    if (group == Emoji.Group.NATIVE)
      entries = _platform.entries(getContext().getAssets());
    else if (group == Emoji.Group.RECENT)
    {
      entries = new ArrayList<>(_lastUsed.keySet());
      Collections.sort(entries, (a, b) -> Integer.compare(_lastUsed.get(b), _lastUsed.get(a)));
    }
    else
      entries = Emoji.getEmojisByGroup(group);
    if (group != Emoji.Group.NATIVE)
      _unicodeGroup = group;
    _physicalPosition = 0;
    setAdapter(new EmojiAdapter(entries, false));
    setSelection(0);
    updateGroupBar();
  }

  void setPlatform(NativeEmoji platform)
  {
    if (_platform == platform)
      return;
    _platform = platform;
    setEmojiGroup(_platform == null ? _unicodeGroup : Emoji.Group.NATIVE);
  }

  void open(NativeEmoji platform)
  {
    setPlatform(platform);
    if (_platform != null && (_group != Emoji.Group.NATIVE || _choosingVariants))
      setEmojiGroup(Emoji.Group.NATIVE);
    else
      closeVariants();
  }

  private boolean openVariants(Emoji emoji)
  {
    _categoryPosition = getFirstVisiblePosition();
    _choosingVariants = true;
    _physicalPosition = 0;
    setAdapter(new EmojiAdapter(emoji.variants, true));
    setSelection(0);
    updateGroupBar();
    return true;
  }

  void closeVariants()
  {
    if (!_choosingVariants)
      return;
    setEmojiGroup(_group);
    setSelection(_categoryPosition);
    _physicalPosition = _categoryPosition;
  }

  void setPhysicalPicker(Runnable onSelection)
  {
    _physicalSelectionCallback = onSelection;
    boolean physical = onSelection != null;
    setBackground(physical ? null : _paneBackground);
    int height = getResources().getDimensionPixelSize(R.dimen.emoji_grid_height);
    if (physical)
    {
      float density = getResources().getDisplayMetrics().density;
      height = Math.min(height, Math.round(getResources().getConfiguration().screenHeightDp * density / 2));
    }
    ViewGroup.LayoutParams params = getLayoutParams();
    params.height = height;
    setLayoutParams(params);
    if (onSelection != null && getCount() == 0)
      stepPhysicalGroup(1);
    if (onSelection != null)
      showPhysicalSelection(0);
    else
      invalidate();
    updateGroupBar();
  }

  void stepPhysicalGroup(int direction)
  {
    Emoji.Group[] groups = Emoji.Group.values();
    int index = _group.ordinal();
    do
    {
      index = (index + direction + groups.length) % groups.length;
    }
    while ((groups[index] == Emoji.Group.NATIVE && _platform == null)
        || (groups[index] == Emoji.Group.RECENT && _lastUsed.isEmpty()));
    setEmojiGroup(groups[index]);
  }

  void movePhysicalSelection(int horizontal, int vertical)
  {
    if (getCount() == 0)
      return;
    int columns = Math.max(1, getNumColumns());
    int position = _physicalPosition + horizontal + vertical * columns;
    if (vertical > 0 && _physicalPosition / columns == (getCount() - 1) / columns)
      return;
    if (position >= 0)
      showPhysicalSelection(Math.min(position, getCount() - 1));
  }

  private void showPhysicalSelection(int position)
  {
    _physicalPosition = position;
    if (position < getFirstVisiblePosition() || position > getLastVisiblePosition())
      setSelection(position);
    invalidate();
  }

  @Override
  protected void dispatchDraw(Canvas canvas)
  {
    super.dispatchDraw(canvas);
    if (_physicalSelectionCallback == null)
      return;
    View cell = getChildAt(_physicalPosition - getFirstVisiblePosition());
    if (cell == null)
      return;
    float inset = _physicalHighlight.getStrokeWidth();
    canvas.drawRoundRect(cell.getLeft() + inset, cell.getTop() + inset,
        cell.getRight() - inset, cell.getBottom() - inset,
        2 * inset, 2 * inset, _physicalHighlight);
  }

  void confirmPhysicalSelection()
  {
    if (_physicalPosition < getCount())
      select((Emoji)getAdapter().getItem(_physicalPosition));
  }

  private void updateGroupBar()
  {
    if (getParent() instanceof ViewGroup)
    {
      EmojiGroupButtonsBar bar = ((ViewGroup)getParent()).findViewById(R.id.emoji_group_bar);
      if (bar != null)
        bar.update(_group, _choosingVariants, _platform, _physicalSelectionCallback != null);
    }
  }

  private void select(Emoji emoji)
  {
    if (!emoji.isNative())
    {
      Integer used = _lastUsed.get(emoji);
      _lastUsed.put(emoji, used == null ? 1 : used + 1);
      saveLastUsed();
      closeVariants();
    }
    Config.globalConfig().handler.key_up(emoji.kv(), Pointers.Modifiers.EMPTY);
    if (_physicalSelectionCallback != null)
      _physicalSelectionCallback.run();
  }

  private void saveLastUsed()
  {
    SharedPreferences.Editor edit;
    try { edit = emojiSharedPreferences().edit(); }
    catch (Exception e) { return; }
    Set<String> set = new HashSet<>();
    for (Map.Entry<Emoji, Integer> entry : _lastUsed.entrySet())
      set.add(entry.getValue() + "-" + entry.getKey().kv().getString());
    edit.putStringSet(LAST_USE_PREF, set).apply();
  }

  private void loadLastUsed()
  {
    Set<String> saved;
    // Credential-encrypted storage can be unavailable when the device is locked.
    try { saved = emojiSharedPreferences().getStringSet(LAST_USE_PREF, null); }
    catch (Exception e) { return; }
    if (saved == null)
      return;
    for (String entry : saved)
    {
      String[] data = entry.split("-", 2);
      if (data.length != 2)
        continue;
      Emoji emoji = Emoji.getEmojiByString(data[1]);
      if (emoji == null)
        continue;
      try { _lastUsed.put(emoji, Integer.parseInt(data[0])); }
      catch (NumberFormatException e) { /* Ignore an invalid usage count. */ }
    }
  }

  private SharedPreferences emojiSharedPreferences()
  {
    return getContext().getSharedPreferences(LAST_USE_PREF, Context.MODE_PRIVATE);
  }

  private static class EmojiView extends TextView
  {
    private final Paint _markerPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Path _marker = new Path();
    private final float _density;
    private boolean _hasVariants;

    EmojiView(Context context)
    {
      super(context, null, 0, R.style.emojiGridButton);
      _density = getResources().getDisplayMetrics().density;
      setLayoutParams(new AbsListView.LayoutParams(
          ViewGroup.LayoutParams.MATCH_PARENT, Math.round(44 * _density)));
      _markerPaint.setColor(getCurrentTextColor());
      _markerPaint.setAlpha(155);
    }

    void setEmoji(Emoji emoji, boolean hasVariants)
    {
      CharSequence text = emoji.name;
      if (emoji.isNative())
      {
        Bitmap image = emoji.bitmap(getContext());
        if (image != null)
        {
          BitmapDrawable drawable = new BitmapDrawable(getResources(), image);
          int size = Math.round(30 * _density);
          int longest = Math.max(image.getWidth(), image.getHeight());
          drawable.setBounds(0, 0, image.getWidth() * size / longest,
              image.getHeight() * size / longest);
          SpannableString preview = new SpannableString("\uFFFC");
          preview.setSpan(new ImageSpan(drawable), 0, 1, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
          text = preview;
        }
      }
      setTypeface(emoji.isNative() ? Typeface.DEFAULT : Emoji.typeface(getContext()));
      setContentDescription(emoji.isNative() ? emoji.name : null);
      setText(text);
      _hasVariants = hasVariants;
      invalidate();
    }

    @Override
    protected void onSizeChanged(int w, int h, int oldw, int oldh)
    {
      super.onSizeChanged(w, h, oldw, oldh);
      float right = w - 5 * _density;
      float bottom = h - 5 * _density;
      float size = 5 * _density;
      _marker.reset();
      _marker.moveTo(right - size, bottom);
      _marker.lineTo(right, bottom - size);
      _marker.lineTo(right, bottom);
      _marker.close();
    }

    @Override
    protected void onDraw(Canvas canvas)
    {
      super.onDraw(canvas);
      if (_hasVariants)
        canvas.drawPath(_marker, _markerPaint);
    }
  }

  private class EmojiAdapter extends BaseAdapter
  {
    private final List<Emoji> _entries;
    private final boolean _showingVariants;
    private final NativeEmoji _editorPlatform;

    EmojiAdapter(List<Emoji> entries, boolean showingVariants)
    {
      _entries = entries;
      _showingVariants = showingVariants;
      _editorPlatform = _platform;
    }

    @Override
    public int getCount() { return _entries.size(); }

    @Override
    public Emoji getItem(int position) { return _entries.get(position); }

    @Override
    public long getItemId(int position) { return position; }

    @Override
    public View getView(int position, View convertView, ViewGroup parent)
    {
      EmojiView view = convertView == null
          ? new EmojiView(parent.getContext()) : (EmojiView)convertView;
      Emoji emoji = getItem(position);
      boolean hasVariants = !_showingVariants && emoji.variants.size() > 1;
      view.setEmoji(emoji, hasVariants);
      // Let the cell handle taps directly. GridView ignores a first tap when its
      // adapter has changed and layout is still pending.
      view.setOnClickListener(v -> {
        if (!emoji.isNative() || _editorPlatform == _platform)
          select(emoji);
      });
      view.setOnLongClickListener(hasVariants ? v -> openVariants(emoji) : null);
      view.setLongClickable(hasVariants);
      view.setFocusable(false);
      return view;
    }
  }
}
