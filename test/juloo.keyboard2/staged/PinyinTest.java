package juloo.keyboard2.staged;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;
import java.io.BufferedReader;
import java.io.File;
import java.io.FileReader;
import java.util.ArrayList;
import java.util.List;
import org.junit.Test;

public class PinyinTest
{
  static Pinyin typed(String keys)
  {
    Pinyin pinyin = new Pinyin();
    for (int i = 0; i < keys.length(); i++)
      assertTrue("key not consumed: " + keys.charAt(i),
          pinyin.type(String.valueOf(keys.charAt(i))));
    return pinyin;
  }

  static List<Composer.Candidate> candidates(String keys)
  {
    return typed(keys).candidates();
  }

  /** The text of every candidate. */
  static List<String> texts(List<Composer.Candidate> candidates)
  {
    List<String> out = new ArrayList<String>();
    for (Composer.Candidate c : candidates)
      out.add(c.text);
    return out;
  }

  static List<String> texts(String keys)
  {
    return texts(candidates(keys));
  }

  /** The annotation of the candidate whose text is [text], or [null]. */
  static String annotation_of(List<Composer.Candidate> candidates, String text)
  {
    for (Composer.Candidate c : candidates)
      if (c.text.equals(text))
        return c.annotation;
    return null;
  }

  @Test
  public void candidates_carry_their_tone()
  {
    List<Composer.Candidate> ying = candidates("ying");
    assertEquals("ˊ", annotation_of(ying, "营")); // yíng
    assertEquals("ˇ", annotation_of(ying, "影")); // yǐng
  }

  @Test
  public void compose_letters()
  {
    Pinyin pinyin = typed("ni");
    assertEquals("ni", pinyin.text());
    assertEquals("你", texts("ni").get(0));
  }

  @Test
  public void syllables()
  {
    assertEquals("ni", typed("ni").syllables().get(0));
    assertEquals("[ni, hao]", typed("nihao").syllables().toString());
    assertEquals("[zhong]", typed("zhong").syllables().toString());
    assertEquals("[guo]", typed("guo").syllables().toString());
    // The longest syllable wins when a sequence can be read in several ways,
    // "xian" is both 先 and 西 + 安.
    assertEquals("[xian]", typed("xian").syllables().toString());
    assertEquals("[fang, an]", typed("fangan").syllables().toString());
  }

  @Test
  public void syllables_backtrack_to_a_reading()
  {
    // The longest syllable at a position does not always leave a valid tail:
    // "hen" is the longest syllable of "heni", but it leaves "i", which is not
    // a syllable. The reading is "he" followed by "ni", so a parser that only
    // ever takes the longest syllable reads nothing at all here.
    assertEquals("[he, ni]", typed("heni").syllables().toString());
  }

  @Test
  public void every_two_syllable_combination_parses() throws Exception
  {
    // A concatenation of two syllables is always a sequence of syllables, so
    // syllables() must never fail on one. Reading only the longest syllable at
    // each position (no backtracking) failed on 1252 of these pairs, "heni"
    // among them. The list is the committed table that PinyinData is generated
    // from, so this walks what the composer actually knows.
    File table = new File("srcs/pinyin/syllables.txt");
    assertTrue("missing " + table.getAbsolutePath(), table.exists());
    List<String> syllables = new ArrayList<String>();
    BufferedReader in = new BufferedReader(new FileReader(table));
    try
    {
      String line;
      while ((line = in.readLine()) != null)
      {
        // The table opens with comment lines.
        if (line.length() == 0 || line.charAt(0) == '#')
          continue;
        int sep = line.indexOf(' ');
        if (sep > 0)
          syllables.add(line.substring(0, sep));
      }
    }
    finally
    {
      in.close();
    }
    assertTrue("table read as empty", syllables.size() > 300);
    for (String first : syllables)
      for (String second : syllables)
        assertFalse("did not parse: " + first + "+" + second,
            typed(first + second).syllables().isEmpty());
  }

  @Test
  public void candidates_of_several_syllables()
  {
    assertEquals("你好", texts("nihao").get(0));
    assertEquals("中国", texts("zhongguo").get(0));
    assertEquals("你号", texts("nihao").get(1));
  }

  @Test
  public void prefixes_keep_offering_candidates()
  {
    // "n" is a syllable of its own (嗯) and also the beginning of ni, na,
    // nian, ... ; "nih" reads as ni followed by an unfinished h*. Candidates
    // are offered throughout instead of only once a syllable is complete.
    assertTrue(candidates("n").size() > 1);
    assertTrue(texts("n").contains("你"));
    assertTrue(candidates("nih").size() > 0);
    // No syllable starts with "xyz", so there is nothing to offer.
    assertEquals(0, candidates("xyz").size());
    assertEquals(0, candidates("").size());
  }

  @Test
  public void umlaut_is_typed_with_v()
  {
    assertEquals("女", texts("nv").get(0));
    assertTrue(texts("lv").contains("绿"));
    // After j, q and x the umlaut is written with a plain u.
    assertEquals("去", texts("qu").get(0));
  }

  @Test
  public void a_character_is_offered_once()
  {
    List<String> ying = texts("ying");
    // The list is not capped: 英 is reachable, it is not among the three most
    // frequent characters of "ying".
    assertTrue(ying.size() > 3);
    assertTrue(ying.contains("英"));
    assertEquals("应", ying.get(0));
    /* The traditional spelling used to be appended after all the simplified
       ones. Both spellings of the same character next to each other is what a
       user calls a row that repeats itself, and this layout writes simplified
       characters; Zhuyin is the way to type the traditional ones. */
    assertFalse(ying.contains("應"));
  }

  @Test
  public void a_row_that_repeats_its_beginning_is_shortened()
  {
    List<Composer.Candidate> cs = new ArrayList<Composer.Candidate>();
    cs.add(new Composer.Candidate("你好"));
    cs.add(new Composer.Candidate("你好吗"));
    cs.add(new Composer.Candidate("你好啊"));
    cs.add(new Composer.Candidate("你好呀"));
    cs.add(new Composer.Candidate("你好坏"));
    cs.add(new Composer.Candidate("你好毒"));
    assertEquals(4, Pinyin.tidy(cs).size());
    /* A candidate that answers only the beginning of the pinyin is the way a
       long word is typed part by part, it is never counted as a repeat. */
    cs.add(new Composer.Candidate("你", null, 2));
    assertEquals(5, Pinyin.tidy(cs).size());
  }

  @Test
  public void a_traditional_spelling_is_not_repeated_next_to_the_simplified_one()
  {
    List<Composer.Candidate> cs = new ArrayList<Composer.Candidate>();
    cs.add(new Composer.Candidate("中国"));
    cs.add(new Composer.Candidate("中國"));
    List<String> kept = texts(Pinyin.tidy(cs));
    assertEquals(1, kept.size());
    assertEquals("中国", kept.get(0));
  }

  @Test
  public void a_candidate_can_answer_the_beginning_of_the_pinyin()
  {
    Pinyin pinyin = typed("nihao");
    Composer.Candidate ni = null;
    for (Composer.Candidate c : pinyin.candidates())
      if (c.text.equals("你") && c.consumed > 0)
        ni = c;
    assertNotNull(ni);
    // "您" covers "ni", the "hao" is left to be composed.
    assertEquals(2, ni.consumed);
    assertEquals("hao", pinyin.accept(ni));
    assertEquals("hao", pinyin.text());
    assertEquals("好", pinyin.candidates().get(0).text);
    // The caret follows the letters that were taken.
    assertTrue(pinyin.backspace());
    assertEquals("ha", pinyin.text());
  }

  @Test
  public void backspace_and_reset()
  {
    Pinyin pinyin = typed("nih");
    assertTrue(pinyin.backspace());
    assertEquals("ni", pinyin.text());
    assertEquals("你", pinyin.candidates().get(0).text);
    pinyin.reset();
    assertEquals("", pinyin.text());
    assertFalse(pinyin.backspace());
  }

  @Test
  public void caret_edits_the_middle_of_the_pinyin()
  {
    Pinyin pinyin = typed("nihao");
    assertEquals(5, pinyin.cursor());
    assertTrue(pinyin.move_cursor(-3)); // ni|hao
    assertEquals(2, pinyin.cursor());
    assertTrue(pinyin.backspace());     // delete the 'i' before the caret
    assertEquals("nhao", pinyin.text());
    assertTrue(pinyin.type("i"));       // insert at the caret
    assertEquals("nihao", pinyin.text());
    assertEquals(2, pinyin.cursor());
    // Candidates still come from the whole string, not from the caret side.
    assertTrue(texts(pinyin.candidates()).contains("你好"));
  }

  @Test
  public void caret_stays_within_the_string()
  {
    Pinyin pinyin = typed("ni");
    assertFalse(pinyin.move_cursor(5)); // already clamped at the end
    assertEquals(2, pinyin.cursor());
    assertTrue(pinyin.move_cursor(-5)); // clamped at the start
    assertEquals(0, pinyin.cursor());
    assertFalse(pinyin.backspace());    // nothing before the caret
    assertEquals("ni", pinyin.text());
    assertTrue(pinyin.type("h"));       // inserting at the front
    assertEquals("hni", pinyin.text());
    assertEquals(1, pinyin.cursor());
    pinyin.reset();
    assertEquals(0, pinyin.cursor());
  }

  @Test
  public void other_keys_are_not_consumed()
  {
    Pinyin pinyin = typed("ni");
    assertFalse(pinyin.type("1"));
    assertFalse(pinyin.type(" "));
    assertFalse(pinyin.type("n i"));
    assertEquals("ni", pinyin.text());
  }

  @Test
  public void upper_case_letters_are_accepted()
  {
    // Auto capitalisation must not break pinyin.
    assertEquals("你", texts("Ni").get(0));
    assertEquals("nihao", typed("Nihao").text());
  }

  @Test
  public void an_apostrophe_splits_the_syllables()
  {
    // "xian" is one syllable, "xi'an" is two: the separator overrules the
    // automatic reading. The built-in table has no word list, so the two parts
    // are offered as the candidates of each part.
    assertTrue(texts("xian").contains("先"));
    List<String> split = texts("xi'an");
    assertTrue(split.toString(), split.contains("西"));
    assertTrue(split.toString(), split.contains("安"));
    // 先 is a candidate of "xian" as a whole, it must not come back here.
    assertFalse(split.toString(), split.contains("先"));
  }

  @Test
  public void the_separator_is_not_part_of_the_pinyin()
  {
    Pinyin pinyin = typed("xi'an");
    // Shown above the candidates and committed as raw text without the '.
    assertEquals("xian", pinyin.text());
    assertEquals(4, pinyin.cursor());
    // Backspace removes the "n", the separator before it stays until the "a"
    // before it is removed too: the split it marks is still the one typed.
    assertTrue(pinyin.backspace());
    assertEquals("xia", pinyin.text());
    assertTrue(pinyin.backspace());
    assertEquals("xi", pinyin.text());
    assertTrue(pinyin.backspace());
    assertEquals("x", pinyin.text());
    // A separator at the start or repeated says nothing, the letters stay.
    assertEquals("a", typed("'a").text());
    assertEquals("an", typed("'an").text());
    assertEquals("an", typed("a''n").text());
  }

  @Test
  public void a_separator_where_the_reading_is_already_sure_is_ignored()
  {
    // "ni|hao" can only split this way, an apostrophe there says nothing.
    assertEquals("nihao", typed("ni'hao").text());
    // Behind an "n" or an "r" no syllable goes on, the split is implied.
    assertEquals("mena", typed("men'a").text());
    assertEquals("pingerqi", typed("ping'erqi").text());
    assertEquals("们啊", texts("men'a").get(0));
  }

  @Test
  public void a_part_of_a_split_composition_can_be_taken_alone()
  {
    Pinyin pinyin = typed("xi'an");
    Composer.Candidate xi = null;
    for (Composer.Candidate c : pinyin.candidates())
      if (c.text.equals("西") && c.consumed > 0)
        xi = c;
    assertNotNull(xi);
    assertEquals(2, xi.consumed);
    // The separator goes with the part it ended.
    assertEquals("an", pinyin.accept(xi));
    assertEquals("安", pinyin.candidates().get(0).text);
  }

  @Test
  public void the_last_part_of_a_split_comes_first()
  {
    // The characters of "xi" are picked after seeing where the split leads:
    // the part that completes the composition is offered first.
    List<String> ts = texts("xi'an");
    assertTrue(ts.toString(), ts.indexOf("安") < ts.indexOf("西"));
    // Both parts stay offerable, "西" is how the composition is taken apart.
    assertTrue(ts.toString(), ts.contains("西"));
    assertTrue(ts.toString(), ts.contains("安"));
  }
}
