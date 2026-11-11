package juloo.keyboard2.staged;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Type Chinese characters by writing Hanyu Pinyin on a latin layout, for
    example `ni` for 你 and `nihao` for 你好.

    The keys typed so far are held as composing text and translated to
    characters that are offered as candidates. Tone marks are not typed: the
    table is ordered by frequency and tones are ignored, so `ma` offers 吗 妈 马
    …. The traditional variant of the characters is not offered, this input
    method is used with Simplified Chinese; see [Zhuyin] for the traditional
    characters.

    A half-width apostrophe splits the syllables by hand, the way the
    standard IMEs do: "fan'gan" is 反 + 感 and "fang'an" is 方 + 安, while
    "fangan" is read by the automatic reading and comes out as 反感. An
    apostrophe is only a separator, it is never committed nor shown. */
public final class Pinyin implements Composer
{
  /** Splits the syllables, as in "xi'an". The ASCII version is what the
      keyboard sends; the typographic one is what a pasted text carries. */
  static final char SEPARATOR = '\'';
  static final char SEPARATOR_TYPOGRAPHIC = '\u2019';
  /** Splits can be nested, "zhong'guo'ren" has three parts. */
  private static final int MAX_PARTS = 8;
  /** The longest pinyin syllable, "zhuang", "chuang" and "shuang". */
  private static final int MAX_SYLLABLE = 6;

  /** Guard on prefix expansion. A single letter can reach dozens of
      syllables; building every candidate on each keystroke stalled the
      keyboard. The exact reading itself is never capped. */
  private static final int MAX_PREFIX_CANDIDATES = 160;
  /** Candidates taken from each of the extra syllables. */
  private static final int PER_SYLLABLE_LIMIT = 16;
  /** Words taken from the bundled pinyin dictionary for one composition. */
  private static final int WORD_LIMIT = 60;
  /** Characters shown for one part of a split composition, the most frequent
      ones. A syllable has up to eighty characters and only the first ten are
      ever meant; the row shows the parts together. */
  private static final int PART_CHARACTER_LIMIT = 10;
  /** Candidates offered for a syllable boundary the composition can be cut at,
      so that a long word can be typed part by part. */
  private static final int SPLIT_LIMIT = 12;
  /** How many boundaries are offered as cuts. The shallowest ones are the ones
      a word is broken at in practice ("zhong" and "zhongguo" for
      "zhongguoren"); going deeper only fills the row. */
  private static final int MAX_SPLITS = 2;
  /** Candidates that may start with the same character. The dictionary lists
      "nihao" words by weight, so the row used to be twenty entries beginning
      with 你 (你好吗 你好啊 你好坏 你号码 你毫无 ...) and looked like one entry
      repeated. Words that answer only part of the pinyin are not counted: 你
      is how "nihao" is typed as 你 + 好. */
  private static final int SAME_FIRST_LIMIT = 4;

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
      if (!is_letter(c) && !is_separator(c))
        return false;
    }
    /* A separator where the syllables already end by themselves, at the very
       start, or twice in a row, says nothing: dropping it keeps "ni'" from
       looking like it does something. */
    for (int i = 0; i < s.length(); i++)
    {
      char c = s.charAt(i);
      if (!is_separator(c))
      {
        /* Auto capitalisation can turn the first letter of a sentence into an
           upper case letter, pinyin itself is always lower case. */
        _keys.insert(_cursor++, Character.toLowerCase(c));
        continue;
      }
      if (_keys.length() == 0 || _cursor == 0
          || _keys.charAt(_cursor - 1) == SEPARATOR)
        continue;
      /* A separator is kept wherever it can change the reading: after a
         syllable that could go on ("xian" against "xi'an", "fangan" against
         "fang'an"). Behind an "n" or an "r" it changes nothing - no syllable
         goes on with those letters followed by a vowel - so "men'a" is stored
         as "mena" and still reads 们 + 啊. */
      char before = _keys.charAt(_cursor - 1);
      if (_cursor >= 2 && (before == 'n' || before == 'r'))
        continue;
      _keys.insert(_cursor++, SEPARATOR);
    }
    /* The key was a letter or a separator, it was consumed even when the
       separator ended up dropped: sending it as text would put a "'" into the
       editor in the middle of the pinyin. */
    return true;
  }

  private static boolean is_letter(char c)
  {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
  }

  private static boolean is_separator(char c)
  {
    return c == SEPARATOR || c == SEPARATOR_TYPOGRAPHIC;
  }

  @Override
  public boolean backspace()
  {
    while (_cursor > 0)
    {
      char c = _keys.charAt(_cursor - 1);
      /* The separator only says where the syllables split, it is never meant
         to be visible on its own: removing the letter before it removes both,
         "xi'" backspaced once leaves "xi". */
      _keys.deleteCharAt(--_cursor);
      if (!is_separator(c))
        return true;
    }
    return false;
  }

  /** The letters typed, without the separators: they say where the syllables
      split, they are not part of the pinyin. This is what the candidates row
      shows above the candidates and what the editor receives when the
      composition is committed as raw text. */
  @Override
  public String text()
  {
    StringBuilder b = new StringBuilder(_keys.length());
    for (int i = 0; i < _keys.length(); i++)
    {
      char c = _keys.charAt(i);
      if (!is_separator(c))
        b.append(c);
    }
    return b.toString();
  }

  /** The caret, counted in letters: it never sits between a separator and the
      letter before it. */
  @Override
  public int cursor()
  {
    int n = 0;
    for (int i = 0; i < _cursor && i < _keys.length(); i++)
      if (!is_separator(_keys.charAt(i)))
        n++;
    return n;
  }

  @Override
  public boolean move_cursor(int d)
  {
    int c = Math.max(0, Math.min(_keys.length(), _cursor + d));
    /* A separator is not a letter the caret can be moved to: stepping left
       from the first letter after it skips it. */
    while (c > 0 && c < _keys.length() && is_separator(_keys.charAt(c)))
      c += (d > 0) ? 1 : -1;
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
    List<Composer.Candidate> out = new ArrayList<Composer.Candidate>();
    /* The syllables of the parts read so far, and the ones of the unfinished
       part that is being typed right now. */
    List<String> done = new ArrayList<String>();
    int part_start = 0;
    /* The word list knows the spelling without the separators, so the whole
       composition is read with the syllables of the parts joined back
       together: "xi'an" is looked up as "xian" and the words are told apart
       by the number of syllables they are spelled with. */
    boolean is_split = keys.indexOf(SEPARATOR) >= 0;
    /* The first part carries the whole row, the ones after it only fill in
       their own words: the part the user is working on is the first one. */
    int word_limit = WORD_LIMIT;
    int prefix_limit = MAX_PREFIX_CANDIDATES;
    for (int p = 0; p < MAX_PARTS; p++)
    {
      int sep = keys.indexOf(SEPARATOR, part_start);
      boolean end = sep < 0;
      String part = end ? keys.substring(part_start)
          : keys.substring(part_start, sep);
      if (part.length() == 0)
        break;
      /* A part that ends at a separator is spelled out in full, the user has
         said where it stops. The last part is still being typed and may end
         on a half written syllable. */
      Reading r = reading(part, !end);
      if (r == null)
        break;
      done.addAll(r.head);
      done.add(r.tail);
      /* The words of the whole composition belong to the last part: it is the
         one that completes it. A part in the middle only offers its
         characters. */
      boolean completes = end && PinyinData.chars(r.tail) != null;
      int before = out.size();
      add_part(r, join(done), done.size(), is_split, completes, word_limit,
          prefix_limit, out);
      /* The part that completes the composition comes first: it is what says
         where the split leads ("西 安" for "xi'an") and the characters of the
         earlier parts are the ones the user picks from before that. */
      if (completes && before > 0)
      {
        List<Composer.Candidate> last_part =
          new ArrayList<Composer.Candidate>(out.subList(before, out.size()));
        out.subList(before, out.size()).clear();
        out.addAll(0, last_part);
      }
      /* Every part is read even when the row is already full: the words of a
         part are typed on purpose, dropping them loses what the split was for.
         A long row is what a split composition is, the row pages. */
      word_limit = Math.max(1, WORD_LIMIT / 3);
      prefix_limit = Math.max(1, MAX_PREFIX_CANDIDATES / 3);
      part_start = sep + 1;
      if (end)
        break;
    }
    return tidy(out);
  }

  private static String join(List<String> syllables)
  {
    StringBuilder b = new StringBuilder();
    for (String s : syllables)
      b.append(s);
    return b.toString();
  }

  /** The candidates of one part of the composition. [whole] is the reading of
      everything that was typed, which is what the word list is asked about.
      [answers_all] is set for the last part of a composition whose syllables
      are all complete: it is the only one whose candidates answer the whole
      composition. */
  private void add_part(Reading r, String whole, int whole_syllables,
      boolean separated, boolean answers_all, int word_limit, int prefix_limit,
      List<Composer.Candidate> out)
  {
    Set<String> seen = new HashSet<String>();
    for (Composer.Candidate c : out)
      seen.add(c.text);
    boolean tail_complete = PinyinData.chars(r.tail) != null;
    /* The candidates built from the reading itself go at the end of the part,
       the words in front of them. They are marked as split candidates when
       they do not answer the whole composition, so that the row does not take
       them for a repetition of the last one. */
    int part_start = out.size();
    /* The syllables of this part, which are what the word list is read with
       when the composition is split. */
    List<String> exact = new ArrayList<String>(r.head);
    exact.add(r.tail);
    /* The exact reading, when the last syllable is already complete.

       A part of a split composition only shows the beginning of its
       characters: the rest of them are rare ones (西 系 细 习 席 息 喜 吸 希 洗
       戏 then 袭 悉 嘻 稀 熙 夕 ...) and the row belongs to every part at once,
       so an unbounded list would bury the words of the other parts. */
    if (tail_complete)
    {
      int limit = separated ? PART_CHARACTER_LIMIT : Integer.MAX_VALUE;
      int added = 0;
      for (Composer.Candidate c : Syllables.candidates(exact, null, false))
      {
        if (added >= limit)
          break;
        if (seen.add(c.text))
        {
          out.add(c);
          added++;
        }
      }
    }
    /* The word list is queried with the raw letters typed, so an unfinished
       "nih" already offers 你好. Words go in front of the single characters:
       they are what a full pinyin spelling is usually for.

       The list only knows the spelling without the separators, so a split
       composition finds its syllables there too - but as whole-syllable words
       that the user has just overruled ("xian" gives 先, not 西 + 安). Only
       the words written with exactly as many characters as the part has
       syllables are kept: 西安 is 2 characters for 2 syllables, 先 is one.

       A part that does not end the composition offers no word of its own: the
       characters below are what is picked from it, and the word list of its
       spelling would only add rare homophone words (戏 惜 袭 悉 ...). The
       words of the whole composition come from the part that ends it. */
    WordSource ws = word_source;
    if (ws != null && answers_all)
    {
      String code = whole;
      List<Composer.Candidate> word_cs = new ArrayList<Composer.Candidate>();
      for (String w : ws.words(code, word_limit))
        /* A word spelled with one syllable per character is the only kind a
           split composition can want: see the comment above. Unsplitted, the
           length says nothing and every word is taken. */
        if ((!separated || w.length() == whole_syllables) && seen.add(w))
          word_cs.add(new Composer.Candidate(w));
      if (word_cs.size() > 0)
      {
        /* The syllable combinations above also produce the most common words
           (你好, 中国, 我). Remove those copies from the synthesized list
           instead of dropping the dictionary word: the dictionary has the
           frequency order, and dropping its copy used to leave only long-tail
           phrases such as 你好啊 at the front of the row. */
        for (Iterator<Composer.Candidate> it =
            out.subList(part_start, out.size()).iterator(); it.hasNext(); )
          if (seen.contains(it.next().text))
            it.remove();
        /* Into the row, not in front of the whole thing: the words of a part
           typed on purpose come first. */
        insert_words(out, part_start, word_cs);
      }
    }
    if (!answers_all)
      for (int i = part_start; i < out.size(); i++)
      {
        Composer.Candidate c = out.get(i);
        if (c.consumed == 0)
          out.set(i, new Composer.Candidate(c.text, c.annotation,
                whole.length()));
      }
    /* Syllable cuts: the running text offers, for each boundary of the reading,
       the candidates that answer only the beginning of it. Accepting one keeps
       the rest of the letters composed, which is how a long word is typed part
       by part - "nihao" is entered as 你 then 好. */
    List<String> full = new ArrayList<String>(r.head);
    /* The last element is only a boundary, it is never read as a syllable: an
       unfinished tail still marks where the cut before it is. */
    full.add(r.tail);
    StringBuilder prefix = new StringBuilder();
    for (int i = 0; i < full.size() - 1 && i < MAX_SPLITS; i++)
    {
      prefix.append(full.get(i));
      List<String> part = full.subList(0, i + 1);
      List<String> split = new ArrayList<String>();
      if (ws != null)
        for (String w : ws.words(prefix.toString(), SPLIT_LIMIT))
          if (!seen.contains(w))
            split.add(w);
      for (Composer.Candidate c : Syllables.candidates(part, null, false))
        if (!seen.contains(c.text) && split.size() < SPLIT_LIMIT)
          split.add(c.text);
      for (String text : split)
        if (seen.add(text))
          out.add(new Composer.Candidate(text, null, prefix.length()));
    }
    /* Prefix reading: every syllable that begins with the unfinished tail
       keeps the composition readable while the user is still typing. It is
       also what a complete but extendable tail needs - "n" is 嗯 and the
       beginning of ni, na, nian ....

       A part of a split composition does not expand: the parts are typed one
       after the other and the row shows them side by side, an expansion there
       would fill it with combinations of the part with syllables the user has
       not typed. */
    if (!separated)
      out.addAll(prefix_candidates(r.head, r.tail, prefix_limit));
  }

  /** Put [words] at [at], where the wording of the part they answer begins. */
  private static void insert_words(List<Composer.Candidate> out, int at,
      List<Composer.Candidate> words)
  {
    while (at < out.size() && out.get(at).consumed > 0)
      at++;
    out.addAll(at, words);
  }

  /** The candidates of every syllable that starts with the unfinished [tail].
      The lists are interleaved so the most frequent character of every
      syllable comes before the second one of any of them, and every one is cut
      down to fit [limit]. */
  private static List<Composer.Candidate> prefix_candidates(List<String> head,
      String tail, int limit)
  {
    List<Composer.Candidate> out = new ArrayList<Composer.Candidate>();
    if (limit <= 0)
      return out;
    Set<String> seen = new HashSet<String>();
    List<List<Composer.Candidate>> more = new ArrayList<List<Composer.Candidate>>();
    for (String s : PinyinData.syllables_with_prefix(tail))
    {
      if (s.equals(tail))
        continue;
      List<String> extended = new ArrayList<String>(head);
      extended.add(s);
      List<Composer.Candidate> cs = Syllables.candidates(extended, null, false);
      if (cs.size() > PER_SYLLABLE_LIMIT)
        cs = cs.subList(0, PER_SYLLABLE_LIMIT);
      if (cs.size() > 0)
        more.add(cs);
    }
    int longest = 0;
    for (List<Composer.Candidate> cs : more)
      longest = Math.max(longest, cs.size());
    for (int i = 0; i < longest && out.size() < limit; i++)
      for (List<Composer.Candidate> cs : more)
      {
        if (out.size() >= limit)
          break;
        if (i < cs.size() && seen.add(cs.get(i).text))
          out.add(cs.get(i));
      }
    return out;
  }

  /** Take [c] as entered and keep the letters it does not answer. */
  @Override
  public String accept(Composer.Candidate c)
  {
    int n = (c.consumed <= 0) ? _keys.length()
        : Math.min(c.consumed, _keys.length());
    _keys.delete(0, n);
    _cursor = Math.max(0, _cursor - n);
    /* The separator that ended the part just taken goes with it. */
    while (_keys.length() > 0 && _keys.charAt(0) == SEPARATOR)
      _keys.deleteCharAt(0);
    if (_keys.length() == 0)
    {
      _cursor = 0;
      return "";
    }
    _cursor = Math.min(_cursor, _keys.length());
    /* The letters left are what is shown as composing text: without the
       separator, which is not part of the pinyin. */
    return text();
  }

  /** Remove what makes a row of candidates look like it repeats itself.

      Traditional variants are dropped when the simplified spelling is already
      offered (中国 instead of 中国/中國), and no more than [SAME_FIRST_LIMIT]
      candidates answering the whole composition may begin with the same
      character. The first candidate always survives: it is the one the space
      bar enters. */
  static List<Composer.Candidate> tidy(List<Composer.Candidate> cs)
  {
    Set<String> texts = new HashSet<String>();
    for (Composer.Candidate c : cs)
      texts.add(c.text);
    List<Composer.Candidate> kept = new ArrayList<Composer.Candidate>(cs.size());
    Map<Character, Integer> firsts = new HashMap<Character, Integer>();
    for (Composer.Candidate c : cs)
    {
      String simplified = simplified(c.text);
      if (!simplified.equals(c.text) && texts.contains(simplified))
        continue;
      if (c.consumed == 0 && c.text.length() > 0)
      {
        Character first = Character.valueOf(c.text.charAt(0));
        Integer n = firsts.get(first);
        int count = (n == null) ? 0 : n.intValue();
        if (count >= SAME_FIRST_LIMIT)
          continue;
        firsts.put(first, Integer.valueOf(count + 1));
      }
      kept.add(c);
    }
    return kept;
  }

  /** [s] written with simplified characters only. */
  static String simplified(String s)
  {
    StringBuilder b = new StringBuilder(s.length());
    for (int i = 0; i < s.length(); i++)
      b.append(PinyinData.simplified(s.charAt(i)));
    return b.toString();
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
    List<String> out = new ArrayList<String>();
    for (String part : _keys.toString().split(
          (SEPARATOR == '\'') ? "'" : String.valueOf(SEPARATOR), MAX_PARTS))
    {
      if (part.length() == 0)
        continue;
      List<String> s = syllables(part, 0);
      if (s == null)
        return Collections.emptyList();
      out.addAll(s);
    }
    return out;
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
      prefix. Returns [null] when no syllable starts this way, e.g. "xyz".

      [complete] reads every key as spelled, without looking for the reading
      of an unfinished tail: it is what the parts of a split composition are
      read with, the user has already said where they end. */
  private static Reading reading(String keys, boolean complete)
  {
    List<String> all = syllables(keys, 0);
    if (all != null && all.size() > 0)
      return new Reading(all.subList(0, all.size() - 1),
          all.get(all.size() - 1));
    if (complete)
      return null;
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
