package juloo.keyboard2.staged;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Set;

/** Type Chinese characters by writing Hanyu Pinyin on a latin layout, for
    example `ni` for 你 and `nihao` for 你好.

    The keys typed so far are held as composing text and translated to
    characters that are offered as candidates. Tone marks are not typed: the
    table is ordered by frequency and tones are ignored, so `ma` offers 吗 妈 马
    …. The traditional variant of the characters is not offered, this input
    method is used with Simplified Chinese; see [Zhuyin] for the traditional
    characters. */
public final class Pinyin implements Composer
{
  /** The longest pinyin syllable, "zhuang", "chuang" and "shuang". */
  private static final int MAX_SYLLABLE = 6;

  /** Guard on prefix expansion. A single letter can reach dozens of
      syllables; building every candidate on each keystroke stalled the
      keyboard. The exact reading itself is never capped. */
  private static final int MAX_PREFIX_CANDIDATES = 160;
  /** Candidates taken from each of the extra syllables. */
  private static final int PER_SYLLABLE_LIMIT = 16;
  /** Words taken from the bundled pinyin dictionary for one composition. */
  private static final int WORD_LIMIT = 24;

  /** Optional large word list supplied by the app. Null on a plain JVM, where
      the unit tests exercise the built-in character table only. */
  public static volatile WordSource word_source = null;

  /** Implemented by the app's dictionary, which needs Android resources.
      Keeping it behind an interface leaves this composer free of Android. */
  public interface WordSource
  {
    /** Words whose pinyin code starts with [code], best first. */
    List<String> words(String code, int limit);
  }

  private final StringBuilder _keys = new StringBuilder();
  /** Caret inside [_keys]. The arrow keys move it so a letter in the middle
      of the pinyin can be fixed without retyping the rest. Candidates are
      still computed from the whole string. */
  private int _cursor = 0;

  @Override
  public boolean type(String s)
  {
    if (s.length() == 0)
      return false;
    for (int i = 0; i < s.length(); i++)
    {
      char c = s.charAt(i);
      /* Auto capitalisation can turn the first letter of a sentence into an
         upper case letter, pinyin itself is always lower case. */
      if (!is_letter(c))
        return false;
    }
    for (int i = 0; i < s.length(); i++)
      _keys.insert(_cursor++, Character.toLowerCase(s.charAt(i)));
    return true;
  }

  private static boolean is_letter(char c)
  {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
  }

  @Override
  public boolean backspace()
  {
    if (_cursor == 0)
      return false;
    _keys.deleteCharAt(_cursor - 1);
    _cursor--;
    return true;
  }

  @Override
  public String text()
  {
    return _keys.toString();
  }

  @Override
  public int cursor()
  {
    return _cursor;
  }

  @Override
  public boolean move_cursor(int d)
  {
    int c = Math.max(0, Math.min(_keys.length(), _cursor + d));
    if (c == _cursor)
      return false;
    _cursor = c;
    return true;
  }

  @Override
  public List<Composer.Candidate> candidates()
  {
    String keys = _keys.toString();
    if (keys.length() == 0)
      return Collections.emptyList();
    Reading r = reading(keys);
    if (r == null)
      return Collections.emptyList();
    List<Composer.Candidate> out = new ArrayList<Composer.Candidate>();
    /* The exact reading, when the last syllable is already complete. */
    if (PinyinData.chars(r.tail) != null)
    {
      List<String> exact = new ArrayList<String>(r.head);
      exact.add(r.tail);
      out.addAll(Syllables.candidates(exact, null, false));
    }
    /* The word list is queried with the raw letters typed, so an unfinished
       "nih" already offers 你好. Words go in front of the single characters:
       they are what a full pinyin spelling is usually for. */
    WordSource ws = word_source;
    if (ws != null)
    {
      Set<String> word_texts = new HashSet<String>();
      List<Composer.Candidate> word_cs =
        new ArrayList<Composer.Candidate>();
      for (String w : ws.words(keys, WORD_LIMIT))
        if (word_texts.add(w))
          word_cs.add(new Composer.Candidate(w));
      if (word_cs.size() > 0)
      {
        /* The syllable combinations above also produce the most common words
           (你好, 中国, 我). Remove those copies from the synthesized list
           instead of dropping the dictionary word: the dictionary has the
           frequency order, and dropping its copy used to leave only long-tail
           phrases such as 你好啊 at the front of the row. */
        for (Iterator<Composer.Candidate> it = out.iterator(); it.hasNext(); )
          if (word_texts.contains(it.next().text))
            it.remove();
        out.addAll(0, word_cs);
      }
    }
    /* Prefix reading: every syllable that begins with the unfinished tail
       keeps the composition readable while the user is still typing. The
       lists are interleaved so the most frequent character of every syllable
       comes before the second one of any of them. */
    /* The cached list is already ordered short/common syllables first. */
    List<String> matches = PinyinData.syllables_with_prefix(r.tail);
    List<List<Composer.Candidate>> more = new ArrayList<List<Composer.Candidate>>();
    for (String s : matches)
    {
      if (s.equals(r.tail))
        continue;
      List<String> extended = new ArrayList<String>(r.head);
      extended.add(s);
      List<Composer.Candidate> cs = Syllables.candidates(extended, null, false);
      if (cs.size() > PER_SYLLABLE_LIMIT)
        cs = cs.subList(0, PER_SYLLABLE_LIMIT);
      if (cs.size() > 0)
        more.add(cs);
    }
    Set<String> seen = new HashSet<String>();
    for (Composer.Candidate c : out)
      seen.add(c.text);
    int longest = 0;
    for (List<Composer.Candidate> cs : more)
      longest = Math.max(longest, cs.size());
    for (int i = 0; i < longest && out.size() < MAX_PREFIX_CANDIDATES; i++)
      for (List<Composer.Candidate> cs : more)
      {
        if (out.size() >= MAX_PREFIX_CANDIDATES)
          break;
        if (i < cs.size() && seen.add(cs.get(i).text))
          out.add(cs.get(i));
      }
    return out;
  }

  @Override
  public void reset()
  {
    _keys.setLength(0);
    _cursor = 0;
  }

  /** The syllables spelled by the keys typed so far, or an empty list when the
      keys are not a sequence of complete syllables.

      The longest syllable first, stepping back to a shorter one when the
      longest does not leave a valid tail - the way [Zhuyin.parse] reads a
      Bopomofo sequence. Without the step back, "heni" reads "hen" and then
      stalls on the "i" that is left: reading only the longest syllable at each
      position failed on 1252 of the 396 squared two-syllable combinations. */
  List<String> syllables()
  {
    List<String> out = syllables(_keys.toString(), 0);
    return (out == null) ? Collections.emptyList() : out;
  }

  /** A composition split into the complete syllables before the last one, and
      the last syllable or unfinished prefix. "nih" reads as ["ni"] + "h". */
  private static final class Reading
  {
    final List<String> head;
    final String tail;

    Reading(List<String> head, String tail)
    {
      this.head = head;
      this.tail = tail;
    }
  }

  /** Split [keys] so that [tail] is either a complete syllable or the
      beginning of one. The full reading is preferred; otherwise the longest
      head that parses is kept and the remainder must still be a syllable
      prefix. Returns [null] when no syllable starts this way, e.g. "xyz". */
  private static Reading reading(String keys)
  {
    List<String> complete = syllables(keys, 0);
    if (complete != null && complete.size() > 0)
      return new Reading(complete.subList(0, complete.size() - 1),
          complete.get(complete.size() - 1));
    for (int cut = keys.length() - 1; cut >= 0; cut--)
    {
      List<String> head = syllables(keys.substring(0, cut), 0);
      String tail = keys.substring(cut);
      if (head != null && PinyinData.syllables_with_prefix(tail).size() > 0)
        return new Reading(head, tail);
    }
    return null;
  }

  /** The syllables of [keys] from [i], or [null] when it has no reading. */
  private static List<String> syllables(String keys, int i)
  {
    if (i == keys.length())
      return new ArrayList<String>();
    int max = Math.min(MAX_SYLLABLE, keys.length() - i);
    for (int len = max; len > 0; len--)
    {
      if (PinyinData.chars(keys.substring(i, i + len)) == null)
        continue;
      List<String> rest = syllables(keys, i + len);
      if (rest != null)
      {
        rest.add(0, keys.substring(i, i + len));
        return rest;
      }
    }
    return null;
  }
}
