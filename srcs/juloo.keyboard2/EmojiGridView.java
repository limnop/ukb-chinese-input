package juloo.keyboard2;

import android.content.Context;
import android.content.SharedPreferences;
import android.util.AttributeSet;
import android.view.ContextThemeWrapper;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.widget.BaseAdapter;
import android.widget.GridView;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

public class EmojiGridView extends GridView
{
  public static final int GROUP_LAST_USE = -1;

  private static final String LAST_USE_PREF = "emoji_last_use";

  private List<Emoji> _emojiArray;
  private HashMap<Emoji, Integer> _lastUsed;

  /** Tap detection. [GridView] throws away a tap while its adapter data has
      changed but no layout has run yet: [AbsListView.onTouchEvent] only enters
      [TOUCH_MODE_DOWN] under `if (!mDataChanged)`. That is exactly the state
      left by [setAdapter], which is what opening the emoji pane does, so the
      first tap after entering the pane is ignored and keys start working only
      after a scroll forces a layout. The tap is recognised here instead, which
      does not depend on that internal state. [super.onTouchEvent] still gets
      every event, so scrolling, flinging and the pressed feedback are
      unchanged. */
  private int _tap_slop;
  private float _down_x;
  private float _down_y;
  private int _down_position = INVALID_POSITION;
  private boolean _tap_pending = false;

  /*
   ** TODO: adapt column width and emoji size
   ** TODO: use ArraySet instead of Emoji[]
   */
  public EmojiGridView(Context context, AttributeSet attrs)
  {
    super(context, attrs);
    Emoji.init(context.getResources());
    _tap_slop = ViewConfiguration.get(context).getScaledTouchSlop();
    loadLastUsed();
    setEmojiGroup((_lastUsed.size() == 0) ? 0 : GROUP_LAST_USE);
  }

  public void setEmojiGroup(int group)
  {
    _emojiArray = (group == GROUP_LAST_USE) ? getLastEmojis() : Emoji.getEmojisByGroup(group);
    setAdapter(new EmojiViewAdpater(getContext(), _emojiArray));
    cancel_tap();
  }

  @Override
  public boolean onTouchEvent(MotionEvent ev)
  {
    switch (ev.getActionMasked())
    {
      case MotionEvent.ACTION_DOWN:
        _down_x = ev.getX();
        _down_y = ev.getY();
        _down_position = position_at(_down_x, _down_y);
        _tap_pending = _down_position != INVALID_POSITION;
        break;
      case MotionEvent.ACTION_MOVE:
        if (_tap_pending
            && (Math.abs(ev.getX() - _down_x) > _tap_slop
                || Math.abs(ev.getY() - _down_y) > _tap_slop))
          _tap_pending = false;
        break;
      case MotionEvent.ACTION_UP:
        /* A long press has no meaning on an emoji: skip it rather than
           inserting on the release of a press-and-hold. */
        if (_tap_pending
            && ev.getEventTime() - ev.getDownTime()
               < ViewConfiguration.getLongPressTimeout())
          select(_down_position);
        cancel_tap();
        break;
      case MotionEvent.ACTION_CANCEL:
        cancel_tap();
        break;
    }
    super.onTouchEvent(ev);
    return true;
  }

  private void cancel_tap()
  {
    _tap_pending = false;
    _down_position = INVALID_POSITION;
  }

  /** Position under a local coordinate, [INVALID_POSITION] when there is no
      cell there. [pointToPosition] covers the normal case; the fallback walks
      the visible cells in case the grid has not indexed them yet. */
  private int position_at(float x, float y)
  {
    int pos = pointToPosition((int)x, (int)y);
    if (pos != INVALID_POSITION)
      return pos;
    for (int i = 0; i < getChildCount(); i++)
    {
      View child = getChildAt(i);
      if (x >= child.getLeft() && x < child.getRight()
          && y >= child.getTop() && y < child.getBottom())
        return getFirstVisiblePosition() + i;
    }
    return INVALID_POSITION;
  }

  private void select(int pos)
  {
    if (_emojiArray == null || pos < 0 || pos >= _emojiArray.size())
      return;
    Emoji emoji = _emojiArray.get(pos);
    Config config = Config.globalConfig();
    Integer used = _lastUsed.get(emoji);
    _lastUsed.put(emoji, (used == null) ? 1 : used.intValue() + 1);
    config.handler.key_up(emoji.kv(), Pointers.Modifiers.EMPTY);
    saveLastUsed(); // TODO: opti
  }

  private List<Emoji> getLastEmojis()
  {
    List<Emoji> list = new ArrayList<>(_lastUsed.keySet());
    Collections.sort(list, new Comparator<Emoji>()
        {
          public int compare(Emoji a, Emoji b)
          {
            return _lastUsed.get(b) - _lastUsed.get(a);
          }
        });
    return list;
  }

  private void saveLastUsed()
  {
    SharedPreferences.Editor edit;
    try { edit = emojiSharedPreferences().edit(); }
    catch (Exception _e) { return; }
    HashSet<String> set = new HashSet<String>();
    for (Emoji emoji : _lastUsed.keySet())
      set.add(String.valueOf(_lastUsed.get(emoji)) + "-" + emoji.kv().getString());
    edit.putStringSet(LAST_USE_PREF, set);
    edit.apply();
  }

  private void loadLastUsed()
  {
    _lastUsed = new HashMap<Emoji, Integer>();
    SharedPreferences prefs;
    // Storage might not be available (eg. the device is locked), avoid
    // crashing.
    try { prefs = emojiSharedPreferences(); }
    catch (Exception _e) { return; }
    Set<String> lastUseSet = prefs.getStringSet(LAST_USE_PREF, null);
    if (lastUseSet != null)
      for (String emojiData : lastUseSet)
      {
        String[] data = emojiData.split("-", 2);
        Emoji emoji;
        if (data.length != 2)
          continue ;
        emoji = Emoji.getEmojiByString(data[1]);
        if (emoji == null)
          continue ;
        _lastUsed.put(emoji, Integer.valueOf(data[0]));
      }
  }

  SharedPreferences emojiSharedPreferences()
  {
    return getContext().getSharedPreferences("emoji_last_use", Context.MODE_PRIVATE);
  }

  static class EmojiView extends TextView
  {
    public EmojiView(Context context)
    {
      super(context);
    }

    public void setEmoji(Emoji emoji)
    {
      setText(emoji.kv().getString());
    }
  }

  static class EmojiViewAdpater extends BaseAdapter
  {
    Context _button_context;

    List<Emoji> _emojiArray;

    public EmojiViewAdpater(Context context, List<Emoji> emojiArray)
    {
      _button_context = new ContextThemeWrapper(context, R.style.emojiGridButton);
      _emojiArray = emojiArray;
    }

    public int getCount()
    {
      if (_emojiArray == null)
        return (0);
      return (_emojiArray.size());
    }

    public Object getItem(int pos)
    {
      return (_emojiArray.get(pos));
    }

    public long getItemId(int pos)
    {
      return (pos);
    }

    public View getView(int pos, View convertView, ViewGroup parent)
    {
      EmojiView view = (EmojiView)convertView;

      if (view == null)
        view = new EmojiView(_button_context);
      view.setEmoji(_emojiArray.get(pos));
      return view;
    }
  }
}
