package juloo.keyboard2;

import android.text.InputType;
import static org.junit.Assert.assertEquals;
import org.junit.Test;

public class EditorConfigTest
{
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
