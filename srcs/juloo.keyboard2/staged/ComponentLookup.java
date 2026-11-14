package juloo.keyboard2.staged;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.Reader;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/** U-mode component-pinyin lookup; independent of ordinary pinyin decoding. */
public final class ComponentLookup
{
  private static final class Entry
  {
    final String code, word, reading;
    final int frequency;
    Entry(String[] fields)
    {
      code = fields[0]; word = fields[1]; reading = fields[2];
      frequency = Integer.parseInt(fields[3]);
    }
  }
  private final List<Entry> entries = new ArrayList<Entry>();

  public ComponentLookup(Reader source) throws IOException
  {
    try (BufferedReader reader = new BufferedReader(source))
    {
      String line;
      while ((line = reader.readLine()) != null)
      {
        String[] fields = line.split("\t", -1);
        if (fields.length != 4) throw new IOException("Malformed component dictionary");
        try { entries.add(new Entry(fields)); }
        catch (NumberFormatException e) { throw new IOException("Invalid component frequency", e); }
      }
    }
    Collections.sort(entries, Comparator.comparing(e -> e.code));
  }

  public List<Composer.Candidate> candidates(String input)
  {
    final String code = input.replace("'", "");
    if (code.isEmpty()) return Collections.emptyList();
    int low = 0, high = entries.size();
    while (low < high)
    {
      int mid = (low + high) >>> 1;
      if (entries.get(mid).code.compareTo(code) < 0) low = mid + 1;
      else high = mid;
    }
    List<Entry> matches = new ArrayList<Entry>();
    for (int i = low; i < entries.size() && entries.get(i).code.startsWith(code); i++)
      matches.add(entries.get(i));
    Collections.sort(matches, (a, b) -> {
      int exact = Boolean.compare(b.code.equals(code), a.code.equals(code));
      return exact != 0 ? exact : Integer.compare(b.frequency, a.frequency);
    });
    List<Composer.Candidate> out = new ArrayList<Composer.Candidate>();
    Set<String> seen = new HashSet<String>();
    for (Entry e : matches)
      if (seen.add(e.word))
      {
        out.add(new Composer.Candidate(e.word, e.reading));
        if (out.size() == 1000) break;
      }
    return out;
  }
}
