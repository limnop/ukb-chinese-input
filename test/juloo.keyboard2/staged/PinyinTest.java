package juloo.keyboard2.staged;

import static org.junit.Assert.*;
import java.util.Arrays;
import java.util.List;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;

/** Buffer/decoder contract tests. Real decoding is exercised on Android. */
public class PinyinTest
{
  private Pinyin.Engine previous;
  private FakeEngine decoder;
  private static class FakeEngine implements Pinyin.Engine
  {
    String input, context, learnedInput, learnedWord, learnedContext;
    final List<Composer.Candidate> results = Arrays.asList(
        new Composer.Candidate("西安"), new Composer.Candidate("西", null, 3));
    public List<Composer.Candidate> candidates(String i, String c)
      { input = i; context = c; return results; }
    public void record_selection(String i, String w, String c)
      { learnedInput = i; learnedWord = w; learnedContext = c; }
  }
  @Before public void setup()
  {
    previous = Pinyin.engine;
    decoder = new FakeEngine();
    Pinyin.engine = decoder;
  }
  @After public void restore() { Pinyin.engine = previous; }
  private Pinyin typed(String input)
  {
    Pinyin p = new Pinyin();
    assertTrue(p.type(input));
    return p;
  }
  @Test public void decoderReceivesUnsegmentedInputAndKeepsItsOrder()
  {
    Pinyin p = typed("Xi’An");
    assertSame(decoder.results, p.candidates());
    assertEquals("xi'an", decoder.input);
    assertEquals("xi'an", p.text());
    assertEquals(5, p.cursor());
  }
  @Test public void unavailableEngineHasNoLegacyCandidates()
  {
    Pinyin.engine = null;
    assertTrue(typed("nihao").candidates().isEmpty());
  }
  @Test public void partialSelectionUsesRawOffsetsAndUpdatesContext()
  {
    Pinyin p = typed("xi'an");
    p.set_context("去");
    assertEquals("an", p.accept(decoder.results.get(1)));
    assertEquals("xi'", decoder.learnedInput);
    assertEquals("西", decoder.learnedWord);
    assertEquals("去", decoder.learnedContext);
    p.candidates();
    assertEquals("去西", decoder.context);
    assertEquals(2, p.cursor());
  }
  @Test public void boundarySeparatorIsRemovedWithSelectedPrefix()
  {
    Pinyin p = typed("xi'an");
    assertEquals("an", p.accept(new Composer.Candidate("西", null, 2)));
    assertEquals(2, p.cursor());
  }
  @Test public void wholeSelectionClearsBuffer()
  {
    Pinyin p = typed("xi'an");
    assertEquals("", p.accept(decoder.results.get(0)));
    assertEquals("xi'an", decoder.learnedInput);
    assertEquals(0, p.cursor());
    assertFalse(p.backspace());
  }
  @Test public void editsAtCaretPreserveWholeQuery()
  {
    Pinyin p = typed("nihao");
    assertTrue(p.move_cursor(-3));
    assertTrue(p.backspace());
    assertEquals("nhao", p.text());
    assertTrue(p.type("i"));
    p.candidates();
    assertEquals("nihao", decoder.input);
    assertEquals(2, p.cursor());
    assertTrue(p.move_cursor(-99));
    assertFalse(p.backspace());
    assertTrue(p.move_cursor(99));
    assertFalse(p.move_cursor(1));
  }
  @Test public void separatorsAreEditableAndRejectedTextIsAtomic()
  {
    Pinyin p = typed("xi'");
    assertTrue(p.backspace());
    assertEquals("xi", p.text());
    assertFalse(p.type("an!"));
    assertFalse(p.type(" "));
    assertFalse(p.type(""));
    assertEquals("xi", p.text());
  }
  @Test public void resetAndContextChangesCannotReuseStaleContext()
  {
    Pinyin p = typed("ni");
    p.set_context("昨天");p.candidates();
    p.set_context("今天");p.candidates();
    assertEquals("今天", decoder.context);
    p.reset();p.type("hao");p.candidates();
    assertEquals("", decoder.context);
    assertEquals("hao", decoder.input);
  }
  @Test public void vModeKeepsExpressionDigitsAndOperators()
  {
    Pinyin p = typed("v12.5*(3-1)");
    assertTrue(p.is_v_mode());
    assertTrue(p.is_special_mode());
    p.candidates();
    assertEquals("v12.5*(3-1)", decoder.input);
    assertEquals("", p.accept(new Composer.Candidate("25")));
    assertNull(decoder.learnedWord);
    assertFalse(p.is_special_mode());
    assertFalse(typed("nv").is_v_mode());
    assertFalse(typed("ni").type("2"));
  }
  @Test public void componentSelectionsDoNotLearnComponentCodeAsPinyin()
  {
    Pinyin p = typed("umumu");
    assertTrue(p.is_special_mode());
    assertFalse(p.is_v_mode());
    p.accept(new Composer.Candidate("林"));
    assertNull(decoder.learnedWord);
    assertEquals("",p.text());
  }
  @Test public void modesCanBeDeletedAndReset()
  {
    Pinyin p = typed("v1+");
    assertTrue(p.backspace());assertEquals("v1",p.text());
    p.reset();assertFalse(p.is_special_mode());
    p.type("u");assertTrue(p.backspace());assertFalse(p.is_special_mode());
  }
}
