package juloo.keyboard2.dict;

import android.content.Context;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import juloo.keyboard2.staged.Pinyin;

/** A large pinyin-indexed Chinese word list, bundled with the APK.

    [assets/zh_pinyin.dat] holds one line per entry, `code<TAB>weight<TAB>word`,
    sorted by code and then by descending weight. The whole file is read into a
    byte array plus an array of line offsets: no hash table, no per-word
    objects, and a lookup is a binary search followed by a short scan. That
    keeps a million-word dictionary to roughly its own size in memory.

    Loading happens on a background thread so the first keystroke never waits
    for the file. Until it is ready [Pinyin] falls back to its built-in
    character table. */
public final class ChineseWordDictionary implements Pinyin.WordSource
{
  static final String ASSET = "zh_pinyin.dat";
  /** Hits collected at most for one lookup. */
  static final int SCAN_LIMIT = 400;
  /** Lines read at most, including the ones skipped because their code has
      already contributed enough words. A short prefix can have thousands of
      entries for its first code; without a row bound the lookup would walk
      all of them. */
  static final int ROW_LIMIT = 8000;
  /** Words kept from each extended code. Reading strictly the first lines of
      a prefix range used to spend the whole budget on the code that sorts
      earliest, so a frequent word a few codes later (你好 for "nih") never
      appeared. */
  static final int PER_CODE = 8;
  /** Below this length only exact codes are looked up. A one- or two-letter
      prefix matches hundreds of thousands of words, all from the first
      syllables that happen to sort earliest, which would bury everything the
      user is likely to want. */
  static final int PREFIX_MIN_LENGTH = 3;

  static volatile ChineseWordDictionary _loaded = null;
  static volatile boolean _loading = false;

  final byte[] _data;
  final int[] _line_starts;

  ChineseWordDictionary(byte[] data)
  {
    _data = data;
    int count = 0;
    for (int i = 0; i < data.length; i++)
      if (data[i] == '\n')
        count++;
    _line_starts = new int[count];
    int line = 0, start = 0;
    for (int i = 0; i < data.length && line < count; i++)
      if (data[i] == '\n')
      {
        _line_starts[line++] = start;
        start = i + 1;
      }
  }

  /** Read the word list in the background and register it on [Pinyin]. */
  public static void preload(final Context ctx)
  {
    if (_loaded != null || _loading)
      return;
    _loading = true;
    new Thread(new Runnable()
        {
          @Override
          public void run()
          {
            try
            {
              Context app = ctx.getApplicationContext();
              InputStream in = app.getAssets().open(ASSET);
              /* Allocate once from the asset length. A growing
                 ByteArrayOutputStream plus toByteArray() would briefly hold
                 two or three copies of a 50MB dictionary, which is what
                 makes a large word list risky on a phone. */
              int len = in.available();
              byte[] data;
              if (len > 0)
              {
                data = new byte[len];
                int off = 0;
                while (off < len)
                {
                  int n = in.read(data, off, len - off);
                  if (n < 0)
                    break;
                  off += n;
                }
                if (off < len)
                  data = java.util.Arrays.copyOf(data, off);
              }
              else
              {
                ByteArrayOutputStream out = new ByteArrayOutputStream(1 << 20);
                byte[] buf = new byte[1 << 16];
                int n;
                while ((n = in.read(buf)) > 0)
                  out.write(buf, 0, n);
                data = out.toByteArray();
              }
              in.close();
              ChineseWordDictionary d = new ChineseWordDictionary(data);
              _loaded = d;
              Pinyin.word_source = d;
            }
            catch (IOException e)
            {
              /* The built-in character table remains usable. */
            }
            finally { _loading = false; }
          }
        }, "zh-word-list").start();
  }

  /** Offset of the first line whose code is >= [code]. */
  int lower_bound(String code)
  {
    int lo = 0, hi = _line_starts.length;
    while (lo < hi)
    {
      int mid = (lo + hi) >>> 1;
      if (code_at(mid).compareTo(code) < 0)
        lo = mid + 1;
      else
        hi = mid;
    }
    return lo;
  }

  String code_at(int line)
  {
    int p = _line_starts[line];
    int e = p;
    while (e < _data.length && _data[e] != '\t')
      e++;
    return new String(_data, p, e - p, StandardCharsets.US_ASCII);
  }

  int parse_int(int p, int end)
  {
    int v = 0;
    for (int i = p; i < end; i++)
    {
      byte b = _data[i];
      if (b < '0' || b > '9')
        break;
      v = v * 10 + (b - '0');
    }
    return v;
  }

  @Override
  public List<String> words(String code, int limit)
  {
    if (limit <= 0 || code.length() == 0)
      return Collections.emptyList();
    boolean exact_only = code.length() < PREFIX_MIN_LENGTH;
    List<Hit> hits = new ArrayList<Hit>();
    int i = lower_bound(code);
    int collected = 0, rows = 0;
    String prev_code = null;
    int per_code = 0;
    while (i < _line_starts.length && collected < SCAN_LIMIT
        && rows < ROW_LIMIT)
    {
      int p = _line_starts[i++];
      rows++;
      int t1 = -1, t2 = -1, end = p;
      while (end < _data.length && _data[end] != '\n')
      {
        if (_data[end] == '\t')
        {
          if (t1 < 0) t1 = end;
          else if (t2 < 0) t2 = end;
        }
        end++;
      }
      if (t1 < 0 || t2 < 0)
        break;
      String line_code =
        new String(_data, p, t1 - p, StandardCharsets.US_ASCII);
      if (exact_only ? !line_code.equals(code) : !line_code.startsWith(code))
        break;
      if (!line_code.equals(prev_code))
      {
        prev_code = line_code;
        per_code = 0;
      }
      boolean exact = line_code.equals(code);
      /* An exact spelling is worth reading in full. Extended codes get a
         small quota so the scan also reaches the codes that follow. */
      if (!exact && per_code >= PER_CODE)
        continue;
      String word = new String(_data, t2 + 1, end - t2 - 1,
          StandardCharsets.UTF_8);
      hits.add(new Hit(exact, line_code.length(),
          parse_int(t1 + 1, t2), word));
      per_code++;
      collected++;
    }
    Collections.sort(hits, BEST_FIRST);
    List<String> out = new ArrayList<String>(Math.min(limit, hits.size()));
    Set<String> seen = new HashSet<String>();
    for (Hit h : hits)
    {
      if (!seen.add(h.word))
        continue;
      out.add(h.word);
      if (out.size() >= limit)
        break;
    }
    return out;
  }

  static final class Hit
  {
    final boolean exact;
    final int code_len;
    final int weight;
    final String word;

    Hit(boolean e, int c, int w, String s)
    {
      exact = e;
      code_len = c;
      weight = w;
      word = s;
    }
  }

  /** Exact spelling first, then the most frequent words, then the codes
      closest to what was typed. Sorting by code length alone used to put
      腻害 (nihai) ahead of 你好 (nihao) while typing "nih". */
  static final Comparator<Hit> BEST_FIRST = new Comparator<Hit>()
  {
    @Override
    public int compare(Hit a, Hit b)
    {
      if (a.exact != b.exact)
        return a.exact ? -1 : 1;
      if (a.weight != b.weight)
        return (a.weight > b.weight) ? -1 : 1;
      if (a.code_len != b.code_len)
        return a.code_len - b.code_len;
      return a.word.compareTo(b.word);
    }
  };
}
