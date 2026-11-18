package juloo.keyboard2;

import android.text.InputType;
import static org.junit.Assert.assertEquals;
import org.junit.Test;

public class EditorConfigTest
{
  @Test
  public void chinese_candidate_row_does_not_depend_on_editor_or_preferences()
  {
    EditorConfig editor = new EditorConfig();
    for (int flags = 0; flags < 32; flags++)
    {
      boolean suggestions = (flags & 1) != 0;
      boolean split = (flags & 2) != 0;
      editor.numeric_layout = (flags & 4) != 0;
      editor.password_editor = (flags & 8) != 0;
      editor.should_show_candidates_view = (flags & 16) != 0;
      assertEquals(true, editor.show_candidate_row("pinyin", suggestions, split));
      assertEquals(true, editor.show_candidate_row("zhuyin", suggestions, split));
      assertEquals(suggestions && !split && !editor.numeric_layout && !editor.password_editor,
          editor.show_candidate_row(null, suggestions, split));
    }
  }

  /** The candidates row is hidden exactly on masked fields. It stays up for
      editors that ask for no suggestions (QQ, terminals), which is where a
      Chinese layout shows its letters and candidates. */
  @Test
  public void password_editor_detection()
  {
    assertEquals(true, EditorConfig._is_password(
          InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD));
    assertEquals(true, EditorConfig._is_password(
          InputType.TYPE_CLASS_TEXT
          | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD));
    assertEquals(true, EditorConfig._is_password(
          InputType.TYPE_CLASS_TEXT
          | InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD));
    assertEquals(false, EditorConfig._is_password(
          InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_NORMAL));
    assertEquals(false, EditorConfig._is_password(
          InputType.TYPE_CLASS_TEXT
          | InputType.TYPE_TEXT_VARIATION_SHORT_MESSAGE
          | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS));
    assertEquals(false, EditorConfig._is_password(InputType.TYPE_NULL));
    assertEquals(false, EditorConfig._is_password(InputType.TYPE_CLASS_NUMBER));
  }
}
