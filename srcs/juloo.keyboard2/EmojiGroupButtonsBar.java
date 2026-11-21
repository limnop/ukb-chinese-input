package juloo.keyboard2;

import android.content.Context;
import android.util.AttributeSet;
import android.view.ContextThemeWrapper;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import java.util.EnumMap;
import java.util.Map;

public class EmojiGroupButtonsBar extends LinearLayout
{
  private final Map<Emoji.Group, Button> _buttons = new EnumMap<>(Emoji.Group.class);
  private final LinearLayout _groups;
  private final Button _back;

  public EmojiGroupButtonsBar(Context context, AttributeSet attrs)
  {
    super(context, attrs);
    _groups = new LinearLayout(context);
    addView(_groups, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT));
    for (Emoji.Group group : Emoji.Group.values())
    {
      Button button = makeButton(context, group.icon);
      button.setTypeface(Emoji.typeface(context));
      button.setContentDescription(context.getString(group.label));
      button.setOnClickListener(v -> grid().setEmojiGroup(group));
      _groups.addView(button, new LayoutParams(0, LayoutParams.WRAP_CONTENT, 1));
      _buttons.put(group, button);
    }

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

  void update(Emoji.Group group, boolean choosingVariants)
  {
    _groups.setVisibility(choosingVariants ? GONE : VISIBLE);
    _back.setVisibility(choosingVariants ? VISIBLE : GONE);
    for (Map.Entry<Emoji.Group, Button> entry : _buttons.entrySet())
    {
      Button button = entry.getValue();
      boolean selected = entry.getKey() == group;
      button.setSelected(selected);
      button.setAlpha(selected ? 1f : 0.72f);
    }
  }
}
