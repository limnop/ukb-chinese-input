package juloo.keyboard2.dict;

import android.content.Context;
import android.content.SharedPreferences;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
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
  /** Keep the personalization store small and quick to load. */
  static final int LEARNED_LIMIT = 512;

  static volatile ChineseWordDictionary _loaded = null;
  static volatile boolean _loading = false;

  final byte[] _data;
  final int[] _line_starts;
  final SharedPreferences _learning;
  final LinkedHashMap<String, Integer> _learned =
    new LinkedHashMap<String, Integer>();

  ChineseWordDictionary(byte[] data, Context context)
  {
    _data = data;
    _learning = context.getSharedPreferences("pinyin_learning",
        Context.MODE_PRIVATE);
    Map<String, ?> saved = _learning.getAll();
    for (Map.Entry<String, ?> e : saved.entrySet())
      if (e.getKey().startsWith("w:") && e.getValue() instanceof Integer)
        _learned.put(e.getKey(), (Integer)e.getValue());
    while (_learned.size() > LEARNED_LIMIT)
      _learned.remove(_learned.keySet().iterator().next());
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
              ChineseWordDictionary d = new ChineseWordDictionary(data, app);
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
      if (compare_code(mid, code) < 0)
        lo = mid + 1;
      else
        hi = mid;
    }
    return lo;
  }

  /** Compare a dictionary code to [code] without allocating a String. */
  int compare_code(int line, String code)
  {
    int p = _line_starts[line];
    int i = 0;
    while (p < _data.length && _data[p] != '\t' && i < code.length())
    {
      int a = _data[p++] & 0xff;
      int b = code.charAt(i++);
      if (a != b)
        return a - b;
    }
    if (p < _data.length && _data[p] != '\t')
      return 1;
    return i == code.length() ? 0 : -1;
  }

  boolean code_starts_with(int line, String prefix)
  {
    int p = _line_starts[line];
    for (int i = 0; i < prefix.length(); i++)
      if (p >= _data.length || _data[p++] != prefix.charAt(i))
        return false;
    return true;
  }

  boolean same_code(int a, int b)
  {
    int p = _line_starts[a], q = _line_starts[b];
    while (p < _data.length && q < _data.length
        && _data[p] == _data[q] && _data[p] != '\t')
    {
      p++;
      q++;
    }
    return p < _data.length && q < _data.length
      && _data[p] == '\t' && _data[q] == '\t';
  }

  int code_length(int line)
  {
    int p = _line_starts[line];
    int n = 0;
    while (p < _data.length && _data[p++] != '\t')
      n++;
    return n;
  }

  int learned_count(String code, String word)
  {
    synchronized (_learned)
    {
      Integer count = _learned.get(learning_key(code, word));
      return count == null ? 0 : count.intValue();
    }
  }

  static String learning_key(String code, String word)
  {
    return "w:" + code + ":" + word;
  }

  @Override
  public void record_selection(String code, String word)
  {
    if (code.length() == 0 || word.length() == 0)
      return;
    String key = learning_key(code, word);
    synchronized (_learned)
    {
      int count = _learned.containsKey(key) ? _learned.get(key).intValue() : 0;
      count = Math.min(1000, count + 1);
      _learned.put(key, Integer.valueOf(count));
      SharedPreferences.Editor edit = _learning.edit().putInt(key, count);
      while (_learned.size() > LEARNED_LIMIT)
      {
        String oldest = _learned.keySet().iterator().next();
        _learned.remove(oldest);
        edit.remove(oldest);
      }
      edit.apply();
    }
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
    int prev_line = -1;
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
      boolean exact = compare_code(i - 1, code) == 0;
      if (exact_only ? !exact : !code_starts_with(i - 1, code))
        break;
      if (prev_line < 0 || !same_code(prev_line, i - 1))
      {
        prev_line = i - 1;
        per_code = 0;
      }
      /* An exact spelling is worth reading in full. Extended codes get a
         small quota so the scan also reaches the codes that follow. */
      if (!exact && per_code >= PER_CODE)
        continue;
      String word = new String(_data, t2 + 1, end - t2 - 1,
          StandardCharsets.UTF_8);
      hits.add(new Hit(exact, code_length(i - 1), parse_int(t1 + 1, t2),
          learned_count(code, word), word));
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
    final int learned;
    final String word;

    Hit(boolean e, int c, int w, int l, String s)
    {
      exact = e;
      code_len = c;
      weight = w;
      learned = l;
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
      if (a.learned != b.learned)
        return (a.learned > b.learned) ? -1 : 1;
      if (a.weight != b.weight)
        return (a.weight > b.weight) ? -1 : 1;
      if (a.code_len != b.code_len)
        return a.code_len - b.code_len;
      return a.word.compareTo(b.word);
    }
  };
}
