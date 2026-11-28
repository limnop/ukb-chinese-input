package juloo.keyboard2;

import android.content.Context;
import android.util.AttributeSet;
import android.view.ContextThemeWrapper;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.PopupMenu;
import java.util.EnumMap;
import java.util.Map;

public class EmojiGroupButtonsBar extends LinearLayout
{
  private static final Emoji.Group[] PRIMARY = { Emoji.Group.RECENT, Emoji.Group.FACES,
      Emoji.Group.GESTURES, Emoji.Group.NATURE, Emoji.Group.FOOD };
  private static final Emoji.Group[] MORE = { Emoji.Group.PEOPLE, Emoji.Group.TRAVEL,
      Emoji.Group.ACTIVITIES, Emoji.Group.OBJECTS, Emoji.Group.SYMBOLS, Emoji.Group.FLAGS };

  private final Map<Emoji.Group, Button> _buttons = new EnumMap<>(Emoji.Group.class);
  private final LinearLayout _groups;
  private final Button _native;
  private final Button _more;
  private final Button _back;
  private PopupMenu _moreMenu;

  public EmojiGroupButtonsBar(Context context, AttributeSet attrs)
  {
    super(context, attrs);
    _groups = new LinearLayout(context);
    addView(_groups, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT));
    _native = makeButton(context, "");
    _native.setTextSize(13);
    _native.setOnClickListener(v -> grid().setEmojiGroup(Emoji.Group.NATIVE));
    _groups.addView(_native, new LayoutParams(0, LayoutParams.WRAP_CONTENT, 2));
    _native.setVisibility(GONE);

    for (Emoji.Group group : PRIMARY)
    {
      Button button = makeButton(context, group.icon);
      button.setTypeface(Emoji.typeface(context));
      button.setContentDescription(context.getString(group.label));
      button.setOnClickListener(v -> grid().setEmojiGroup(group));
      _groups.addView(button, new LayoutParams(0, LayoutParams.WRAP_CONTENT, 1));
      _buttons.put(group, button);
    }

    _more = makeButton(context, context.getString(R.string.emoji_group_more));
    _more.setTextSize(13);
    _more.setOnClickListener(v -> showMore());
    _groups.addView(_more, new LayoutParams(0, LayoutParams.WRAP_CONTENT, 1.25f));

    _back = makeButton(context, context.getString(R.string.emoji_variants_back));
    _back.setOnClickListener(v -> grid().closeVariants());
    addView(_back, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT));
    _back.setVisibility(GONE);
  }

  private Button makeButton(Context context, String label)
  {
    Button button = new Button(new ContextThemeWrapper(context, R.style.emojiTypeButton), null, 0);
    button.setText(label);
    return button;
  }

  private EmojiGridView grid()
  {
    return ((ViewGroup)getParent()).findViewById(R.id.emoji_grid);
  }

  private void showMore()
  {
    _moreMenu = new PopupMenu(getContext(), _more);
    for (Emoji.Group group : MORE)
      _moreMenu.getMenu().add(group.label).setOnMenuItemClickListener(item -> {
        grid().setEmojiGroup(group);
        return true;
      });
    _moreMenu.setOnDismissListener(menu -> _moreMenu = null);
    _moreMenu.show();
  }

  @Override
  protected void onDetachedFromWindow()
  {
    if (_moreMenu != null)
      _moreMenu.dismiss();
    super.onDetachedFromWindow();
  }

  private void select(Button button, boolean selected)
  {
    button.setSelected(selected);
    button.setAlpha(selected ? 1f : 0.72f);
  }

  void update(Emoji.Group group, boolean choosingVariants, NativeEmoji platform)
  {
    if (_moreMenu != null)
      _moreMenu.dismiss();
    _groups.setVisibility(choosingVariants ? GONE : VISIBLE);
    _back.setVisibility(choosingVariants ? VISIBLE : GONE);
    _native.setVisibility(platform == null ? GONE : VISIBLE);
    if (platform != null)
      _native.setText(platform.label);
    select(_native, group == Emoji.Group.NATIVE);
    boolean moreSelected = group != Emoji.Group.NATIVE && !_buttons.containsKey(group);
    select(_more, moreSelected);
    _more.setContentDescription(moreSelected ? getResources().getString(
        R.string.emoji_group_more_selected, getResources().getString(group.label)) : null);
    for (Map.Entry<Emoji.Group, Button> entry : _buttons.entrySet())
      select(entry.getValue(), entry.getKey() == group);
  }
}
