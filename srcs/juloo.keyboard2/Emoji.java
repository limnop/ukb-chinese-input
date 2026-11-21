package juloo.keyboard2;

import android.content.Context;
import android.content.res.Resources;
import android.graphics.Typeface;
import android.text.Spannable;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.TextPaint;
import android.text.style.MetricAffectingSpan;
import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.EnumMap;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public final class Emoji
{
  enum Group
  {
    RECENT("🕘", R.string.emoji_group_recent),
    FACES("😀", R.string.emoji_group_faces),
    GESTURES("👋", R.string.emoji_group_gestures),
    PEOPLE("🧑", R.string.emoji_group_people),
    NATURE("🐻", R.string.emoji_group_nature),
    FOOD("🍜", R.string.emoji_group_food),
    TRAVEL("🚗", R.string.emoji_group_travel),
    ACTIVITIES("⚽", R.string.emoji_group_activities),
    OBJECTS("💡", R.string.emoji_group_objects),
    SYMBOLS("💟", R.string.emoji_group_symbols),
    FLAGS("🏁", R.string.emoji_group_flags);

    final String icon;
    final int label;

    Group(String icon, int label)
    {
      this.icon = icon;
      this.label = label;
    }
  }

  private final KeyValue _kv;
  /** Every entry in a family, starting with its classic form. */
  final List<Emoji> variants;

  private Emoji(String text, List<Emoji> variants)
  {
    _kv = new KeyValue(text, KeyValue.Kind.String, 0, 0);
    this.variants = variants;
  }

  public KeyValue kv() { return _kv; }

  private static final Map<Group, List<Emoji>> _groups = new EnumMap<>(Group.class);
  private static final Map<String, Emoji> _by_text = new HashMap<>();
  private static Typeface _font;

  public static Typeface typeface(Context context)
  {
    if (_font == null)
      _font = Typeface.createFromAsset(context.getAssets(), "NotoColorEmoji.ttf");
    return _font;
  }

  /** Apply the emoji font to a single emoji candidate, leaving its number label alone. */
  public static CharSequence styleCandidate(Context context, CharSequence label, String candidate)
  {
    init(context.getResources());
    if (getEmojiByString(candidate) == null)
      return label;

    Typeface font = typeface(context);
    Spannable styled = (label instanceof Spannable) ? (Spannable)label
        : new SpannableString(label);
    styled.setSpan(new MetricAffectingSpan()
      {
        @Override
        public void updateDrawState(TextPaint paint) { paint.setTypeface(font); }

        @Override
        public void updateMeasureState(TextPaint paint) { paint.setTypeface(font); }
      }, label.length() - candidate.length(), label.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
    return styled;
  }

  static void init(Resources res)
  {
    if (!_groups.isEmpty())
      return;

    try (BufferedReader reader = new BufferedReader(new InputStreamReader(
            res.openRawResource(R.raw.emojis), StandardCharsets.UTF_8)))
    {
      List<Emoji> group = null;
      String line;
      while ((line = reader.readLine()) != null)
      {
        if (line.startsWith("# "))
          continue;
        if (line.startsWith("["))
        {
          group = new ArrayList<>();
          _groups.put(Group.valueOf(line.substring(1, line.length() - 1)), group);
          continue;
        }
        List<Emoji> family = new ArrayList<>();
        List<Emoji> variants = Collections.unmodifiableList(family);
        for (String value : line.split("\\t"))
        {
          Emoji emoji = new Emoji(value, variants);
          family.add(emoji);
          _by_text.put(value, emoji);
        }
        group.add(family.get(0));
      }
    }
    catch (IOException e) { Logs.exn("Emoji.init() failed", e); }
  }

  static List<Emoji> getEmojisByGroup(Group group)
  {
    List<Emoji> list = _groups.get(group);
    return list == null ? Collections.emptyList() : list;
  }

  static Emoji getEmojiByString(String value)
  {
    return _by_text.get(value);
  }
}
