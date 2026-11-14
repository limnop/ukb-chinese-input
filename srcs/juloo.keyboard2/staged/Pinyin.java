package juloo.keyboard2.staged;

import java.util.Collections;
import java.util.List;

/** Android composition buffer for WowType. Decoding, segmentation, ranking and
    learning belong to the native engine; this class only edits raw input and
    applies the engine's consumed-input offsets. */
public final class Pinyin implements Composer
{
  public interface Engine
  {
    List<Composer.Candidate> candidates(String input, String context);
    void record_selection(String input, String word, String context);
  }

  /** Published once the native engine finishes loading. No legacy fallback. */
  public static volatile Engine engine;
  private final StringBuilder _keys = new StringBuilder();
  private int _cursor;
  private String _context = "";

  @Override public boolean type(String text)
  {
    if (text.isEmpty()) return false;
    String proposed = new StringBuilder(_keys).insert(_cursor, text).toString();
    boolean calculator = proposed.charAt(0) == 'v' || proposed.charAt(0) == 'V';
    for (int i = 0; i < text.length(); i++)
    {
      char c = text.charAt(i);
      if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
          || c == '\'' || c == '\u2019'
          || (calculator && "0123456789+-*/%^.()×÷".indexOf(c) >= 0))) return false;
    }
    for (int i = 0; i < text.length(); i++)
    {
      char c = text.charAt(i);
      _keys.insert(_cursor++, c == '\u2019' ? '\'' : Character.toLowerCase(c));
    }
    return true;
  }

  public boolean is_v_mode() { return _keys.length() > 0 && _keys.charAt(0) == 'v'; }
  public boolean is_special_mode()
  {
    return _keys.length() > 0 && (_keys.charAt(0) == 'u' || _keys.charAt(0) == 'v');
  }

  @Override public boolean backspace()
  {
    if (_cursor == 0) return false;
    _keys.deleteCharAt(--_cursor);
    return true;
  }

  @Override public String text() { return _keys.toString(); }
  @Override public int cursor() { return _cursor; }

  @Override public boolean move_cursor(int delta)
  {
    int next = Math.max(0, Math.min(_keys.length(), _cursor + delta));
    if (next == _cursor) return false;
    _cursor = next;
    return true;
  }

  @Override public List<Composer.Candidate> candidates()
  {
    Engine current = engine;
    return current == null || _keys.length() == 0
        ? Collections.<Composer.Candidate>emptyList()
        : current.candidates(text(), _context);
  }

  @Override public void set_context(String context)
  {
    _context = context_tail(context);
  }

  private static String context_tail(String text)
  {
    if (text == null) return "";
    int start = Math.max(0, text.length() - 64);
    if (start > 0 && Character.isLowSurrogate(text.charAt(start))) start++;
    return text.substring(start);
  }

  @Override public String accept(Composer.Candidate candidate)
  {
    int consumed = candidate.consumed <= 0 ? _keys.length()
        : Math.min(candidate.consumed, _keys.length());
    Engine current = engine;
    if (current != null && consumed > 0 && !is_special_mode())
      current.record_selection(_keys.substring(0, consumed), candidate.text, _context);
    _context = context_tail(_context + candidate.text);
    _keys.delete(0, consumed);
    _cursor = Math.max(0, _cursor - consumed);
    while (_keys.length() > 0 && _keys.charAt(0) == '\'')
    {
      _keys.deleteCharAt(0);
      _cursor = Math.max(0, _cursor - 1);
    }
    return text();
  }

  @Override public void reset()
  {
    _keys.setLength(0);
    _cursor = 0;
    _context = "";
  }
}
