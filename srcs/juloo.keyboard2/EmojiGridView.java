package juloo.keyboard2;

import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.util.AttributeSet;
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
  private Emoji.Group _group;
  private boolean _choosingVariants;
  private int _categoryPosition;

  public EmojiGridView(Context context, AttributeSet attrs)
  {
    super(context, attrs);
    Emoji.init(context.getResources());
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
    _group = group;
    _choosingVariants = false;
    List<Emoji> entries;
    if (group == Emoji.Group.RECENT)
    {
      entries = new ArrayList<>(_lastUsed.keySet());
      Collections.sort(entries, (a, b) -> Integer.compare(_lastUsed.get(b), _lastUsed.get(a)));
    }
    else
      entries = Emoji.getEmojisByGroup(group);
    setAdapter(new EmojiAdapter(entries, false));
    setSelection(0);
    updateGroupBar();
  }

  private boolean openVariants(Emoji emoji)
  {
    _categoryPosition = getFirstVisiblePosition();
    _choosingVariants = true;
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
  }

  private void updateGroupBar()
  {
    if (getParent() instanceof ViewGroup)
    {
      EmojiGroupButtonsBar bar = ((ViewGroup)getParent()).findViewById(R.id.emoji_group_bar);
      if (bar != null)
        bar.update(_group, _choosingVariants);
    }
  }

  private void select(Emoji emoji)
  {
    Integer used = _lastUsed.get(emoji);
    _lastUsed.put(emoji, used == null ? 1 : used + 1);
    saveLastUsed();
    closeVariants();
    Config.globalConfig().handler.key_up(emoji.kv(), Pointers.Modifiers.EMPTY);
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
      setTypeface(Emoji.typeface(context));
      _density = getResources().getDisplayMetrics().density;
      setLayoutParams(new AbsListView.LayoutParams(
          ViewGroup.LayoutParams.MATCH_PARENT, Math.round(44 * _density)));
      _markerPaint.setColor(getCurrentTextColor());
      _markerPaint.setAlpha(155);
    }

    void setEmoji(Emoji emoji, boolean hasVariants)
    {
      setText(emoji.kv().getString());
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

    EmojiAdapter(List<Emoji> entries, boolean showingVariants)
    {
      _entries = entries;
      _showingVariants = showingVariants;
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
      view.setOnClickListener(v -> select(emoji));
      view.setOnLongClickListener(hasVariants ? v -> openVariants(emoji) : null);
      view.setLongClickable(hasVariants);
      view.setFocusable(false);
      return view;
    }
  }
}
