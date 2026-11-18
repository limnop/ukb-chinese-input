package juloo.keyboard2;

import static org.junit.Assert.*;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import juloo.keyboard2.suggestions.Suggestions;
import org.junit.Test;

public class PinyinBackspaceTest
{
  @Test public void deletingLastLetterRefreshesStripWithDictionarySuggestionsDisabled()
  {
    KeyEventHandler.IReceiver receiver = (KeyEventHandler.IReceiver)Proxy.newProxyInstance(
        getClass().getClassLoader(), new Class<?>[]{KeyEventHandler.IReceiver.class},
        (proxy, method, args) -> {
          if (method.getName().equals("getHandler")) return null;
          throw new AssertionError("Unexpected editor access: " + method.getName());
        });
    List<String> displayed = new ArrayList<>();
    KeyEventHandler handler = new KeyEventHandler(receiver, null);
    handler.set_layout_script("pinyin");
    Suggestions strip = new Suggestions(row -> displayed.add(handler.composing_text()), null);
    handler._suggestions = strip;
    handler._composer.type("ni");
    handler.send_composition();
    handler.handle_backspace();
    handler._composer.move_cursor(-1);
    handler.handle_backspace();
    assertEquals("n", handler.composing_text());
    handler._composer.move_cursor(1);
    handler.handle_backspace();
    assertEquals(Arrays.asList("ni", "n", ""), displayed);
    assertEquals(0, handler.composing_cursor());
    assertTrue(strip.candidates.isEmpty());
  }
}
