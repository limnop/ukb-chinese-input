package juloo.keyboard2;

import android.view.KeyEvent;
import juloo.keyboard2.KeyValue;
import org.junit.Test;
import static juloo.keyboard2.TestUtils.*;
import static org.junit.Assert.*;

public class KeyValueTest
{
  public KeyValueTest() {}

  @Test
  public void equals()
  {
    assertEquals(str("Foo").withSymbol("Symbol"),
        KeyValue.makeMacro("Symbol", new KeyValue[] { str("Foo") }, 0));
    assertEquals(KeyValue.getSpecialKeyByName("tab"),
        KeyValue.keyeventKey(0xE00F, KeyEvent.KEYCODE_TAB, KeyValue.FLAG_KEY_FONT | KeyValue.FLAG_SMALLER_FONT));
    assertEquals(KeyValue.getSpecialKeyByName("tab").withSymbol("t"),
        KeyValue.keyeventKey("t", KeyEvent.KEYCODE_TAB, 0));
    assertEquals(KeyValue.getSpecialKeyByName("tab").withSymbol("tab"),
        KeyValue.keyeventKey("tab", KeyEvent.KEYCODE_TAB, KeyValue.FLAG_SMALLER_FONT));
  }

  /** While a Chinese composition is live, the return key commits the letters
      and must be swallowed. It takes two shapes: a raw ENTER key event, or
      the editor's action key (Done / Go / Search) that replaces it. */
  @Test
  public void chinese_enter_key_shapes()
  {
    assertTrue(KeyEventHandler.is_enter_key(KeyValue.ENTER));
    assertTrue(KeyEventHandler.is_enter_key(KeyValue.makeActionKey("Done")));
    assertFalse(KeyEventHandler.is_enter_key(KeyValue.makeStringKey("a", 0)));
    assertFalse(KeyEventHandler.is_enter_key(KeyValue.getKeyByName("backspace")));
    // Enter ends the composition, a caret key must never be confused with it.
    assertFalse(KeyEventHandler.is_caret_key(KeyValue.ENTER));
  }

  /** The arrows belong to the Chinese composition: left/right move the
      pinyin caret, up/down page the candidates, and digits pick a word. */
  @Test
  public void chinese_navigation_keys()
  {
    assertTrue(KeyEventHandler.is_navigation_key(
        KeyValue.sliderKey(KeyValue.Slider.Cursor_left, 1)));
    assertTrue(KeyEventHandler.is_navigation_key(
        KeyValue.sliderKey(KeyValue.Slider.Cursor_right, 1)));
    assertTrue(KeyEventHandler.is_navigation_key(
        KeyValue.sliderKey(KeyValue.Slider.Cursor_up, 1)));
    assertTrue(KeyEventHandler.is_navigation_key(
        KeyValue.sliderKey(KeyValue.Slider.Cursor_down, 1)));
    assertTrue(KeyEventHandler.is_navigation_key(
        KeyValue.keyeventKey("up", KeyEvent.KEYCODE_DPAD_UP, 0)));
    assertFalse(KeyEventHandler.is_navigation_key(KeyValue.ENTER));
    assertFalse(KeyEventHandler.is_navigation_key(KeyValue.makeStringKey("a", 0)));
    // Only left/right move the caret; up/down page the row.
    assertTrue(KeyEventHandler.is_caret_key(
        KeyValue.sliderKey(KeyValue.Slider.Cursor_left, 1)));
    assertFalse(KeyEventHandler.is_caret_key(
        KeyValue.sliderKey(KeyValue.Slider.Cursor_up, 1)));
    // '1'..'9' pick candidates 1..9, '0' picks the tenth.
    assertEquals(0, KeyEventHandler.candidate_index_of_digit('1'));
    assertEquals(8, KeyEventHandler.candidate_index_of_digit('9'));
    assertEquals(9, KeyEventHandler.candidate_index_of_digit('0'));
    assertEquals(-1, KeyEventHandler.candidate_index_of_digit('a'));
  }

  @Test
  public void numpad_script()
  {
    assertEquals(apply_numpad_script("hindu-arabic"), "٠١٢٣٤٥٦٧٨٩");
    assertEquals(apply_numpad_script("bengali"), "০১২৩৪৫৬৭৮৯");
    assertEquals(apply_numpad_script("devanagari"), "०१२३४५६७८९");
    assertEquals(apply_numpad_script("persian"), "۰۱۲۳۴۵۶۷۸۹");
    assertEquals(apply_numpad_script("gujarati"), "૦૧૨૩૪૫૬૭૮૯");
    assertEquals(apply_numpad_script("kannada"), "೦೧೨೩೪೫೬೭೮೯");
    assertEquals(apply_numpad_script("tamil"), "௦௧௨௩௪௫௬௭௮௯");
  }
  String apply_numpad_script(String script)
  {
    StringBuilder b = new StringBuilder();
    int map = KeyModifier.modify_numpad_script(script);
    for (char c : "0123456789".toCharArray())
      b.append(ComposeKey.apply(map, c).getChar());
    return b.toString();
  }
}
