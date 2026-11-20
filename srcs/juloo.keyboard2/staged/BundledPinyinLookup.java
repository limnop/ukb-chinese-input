package juloo.keyboard2.staged;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.Reader;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** Exact shortcuts for bundled phrases and software names. */
final class BundledPinyinLookup
{
  private final Map<String, List<Composer.Candidate>> _entries = new HashMap<>();

  void load(Reader source) throws IOException
  {
    try (BufferedReader reader = new BufferedReader(source))
    {
      String line;
      while ((line = reader.readLine()) != null)
      {
        if (line.isEmpty() || line.startsWith("#"))
          continue;
        int tab = line.indexOf('\t');
        if (tab < 1 || tab == line.length() - 1 || line.indexOf('\t', tab + 1) >= 0)
          throw new IOException("Malformed bundled pinyin entry");
        String code = line.substring(0, tab);
        String text = line.substring(tab + 1);
        List<Composer.Candidate> candidates = _entries.get(code);
        if (candidates == null)
        {
          candidates = new ArrayList<>();
          _entries.put(code, candidates);
        }
        if (!contains(candidates, text))
          candidates.add(new Composer.Candidate(text));
      }
    }
  }

  List<Composer.Candidate> candidates(String input)
  {
    List<Composer.Candidate> matches = _entries.get(input.replace("'", ""));
    return matches == null ? Collections.emptyList() : Collections.unmodifiableList(matches);
  }

  static boolean contains(List<Composer.Candidate> candidates, String text)
  {
    for (Composer.Candidate candidate : candidates)
      if (candidate.text.equals(text))
        return true;
    return false;
  }
}
