package juloo.keyboard2.staged;

import android.content.Context;
import android.content.SharedPreferences;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/** Android adapter for the vendored WowType Rust pinyin engine. */
public final class WowtypePinyinEngine
{
  private static final String LEXICON_ASSET = "wowtype_lexicon.tsv";
  private static volatile long _engine = 0;
  private static volatile boolean _loading = false;
  private static String _history_path;
  private static String _user_dict_path;
  private static SharedPreferences _prefs;
  private static String _last_context;
  private static boolean _configured = false;
  private static boolean _last_fuzzy;
  private static boolean _last_typo_correction;

  static
  {
    try { System.loadLibrary("wowtype_jni"); }
    catch (UnsatisfiedLinkError e) { android.util.Log.e("WowType", "Native library unavailable", e); }
  }

  private WowtypePinyinEngine() {}

  /** Prepare the data and native engine off the keyboard thread. */
  public static void preload(final Context context)
  {
    if (_engine != 0 || _loading)
      return;
    synchronized (WowtypePinyinEngine.class)
    {
      if (_engine != 0 || _loading)
        return;
      _loading = true;
    }
    final Context app = context.getApplicationContext();
    new Thread(new Runnable()
        {
          @Override public void run()
          {
            try
            {
              File data_dir = new File(app.getFilesDir(), "wowtype");
              if (!data_dir.exists() && !data_dir.mkdirs())
                throw new IOException("Could not create WowType data directory");
              org.json.JSONObject manifest;
              try (InputStream in = app.getAssets().open("wowtype-data.json"))
              {
                java.io.ByteArrayOutputStream bytes = new java.io.ByteArrayOutputStream();
                byte[] buffer = new byte[4096]; int n;
                while ((n = in.read(buffer)) != -1) bytes.write(buffer, 0, n);
                manifest = new org.json.JSONObject(bytes.toString("UTF-8"));
              }
              File lexicon = installAsset(app, data_dir, LEXICON_ASSET, "lexicon.tsv",
                  manifest.getString(LEXICON_ASSET));
              File model = installAsset(app, data_dir, "wowtype_words.lm", "words.lm",
                  manifest.getString("wowtype_words.lm"));
              _history_path = new File(data_dir, "history.tsv").getAbsolutePath();
              _user_dict_path = new File(data_dir, "user.tsv").getAbsolutePath();
              _prefs = juloo.keyboard2.DirectBootAwarePreferences
                  .get_shared_preferences(app);
              long handle = nativeNew(lexicon.getAbsolutePath(), model.getAbsolutePath(),
                  _history_path, _user_dict_path);
              if (handle != 0)
              {
                _engine = handle;
                Pinyin.engine = new NativeEngine();
              }
              else
                android.util.Log.e("WowType", "Engine initialization failed");
            }
            catch (IOException | org.json.JSONException | java.security.NoSuchAlgorithmException | UnsatisfiedLinkError e)
            {
              android.util.Log.e("WowType", "Engine initialization failed", e);
            }
            finally { _loading = false; }
          }
        }, "wowtype-pinyin").start();
  }

  /** Replace bundled data atomically on content changes, preserving user history. */
  private static File installAsset(Context app, File dir, String asset,
      String name, String hash) throws IOException, java.security.NoSuchAlgorithmException
  {
    File file = new File(dir, name), stamp = new File(dir, name + ".sha256");
    if (file.exists() && stamp.exists())
      try (java.io.FileInputStream in = new java.io.FileInputStream(stamp))
      {
        byte[] value = new byte[64];
        if (in.read(value) == 64 && hash.equals(new String(value, "US-ASCII"))) return file;
      }
    File temp = new File(dir, name + ".tmp");
    java.security.MessageDigest digest = java.security.MessageDigest.getInstance("SHA-256");
    try (InputStream in = app.getAssets().open(asset); FileOutputStream out = new FileOutputStream(temp))
    {
      byte[] buffer = new byte[65536]; int n;
      while ((n = in.read(buffer)) != -1) { out.write(buffer, 0, n); digest.update(buffer, 0, n); }
      out.getFD().sync();
    }
    StringBuilder actual = new StringBuilder();
    for (byte b : digest.digest()) actual.append(String.format(java.util.Locale.ROOT, "%02x", b & 255));
    if (!hash.equals(actual.toString())) throw new IOException("Asset checksum mismatch: " + asset);
    if (!temp.renameTo(file)) throw new IOException("Cannot install " + asset);
    File cache = new File(dir, name + ".wcache");
    if (cache.exists() && !cache.delete()) throw new IOException("Cannot invalidate lexicon cache");
    File stampTemp = new File(dir, name + ".sha256.tmp");
    try (FileOutputStream out = new FileOutputStream(stampTemp)) { out.write(hash.getBytes("US-ASCII")); out.getFD().sync(); }
    if (!stampTemp.renameTo(stamp)) throw new IOException("Cannot write data version");
    return file;
  }

  private static List<Composer.Candidate> candidates(String code, String context, int limit)
  {
    long engine = _engine;
    if (engine == 0)
      return Collections.emptyList();
    String[] rows;
    synchronized (WowtypePinyinEngine.class)
    {
      set_context(context);
      rows = nativeInput(engine, code, limit);
    }
    if (rows == null || rows.length == 0)
      return Collections.emptyList();
    List<Composer.Candidate> out = new ArrayList<Composer.Candidate>(rows.length);
    for (String row : rows)
    {
      int tab = row.indexOf('\t');
      if (tab < 0 || tab + 1 >= row.length())
        continue;
      String text = row.substring(tab + 1);
      if (!containsHan(text))
        continue;
      try
      {
        int consumed = Integer.parseInt(row.substring(0, tab));
        /* Composer uses zero for a candidate that answers the whole input;
           preserve positive lengths only for sentence-head candidates. */
        if (consumed >= code.length()) consumed = 0;
        out.add(new Composer.Candidate(text, null, consumed));
      }
      catch (NumberFormatException e) { /* Ignore a malformed native result. */ }
    }
    return out;
  }

  private static void configure(long engine)
  {
    boolean fuzzy = _prefs != null && _prefs.getBoolean("pinyin_fuzzy", false);
    boolean typo = _prefs == null || _prefs.getBoolean("pinyin_typo_correction", true);
    if (!_configured || fuzzy != _last_fuzzy || typo != _last_typo_correction)
    {
      nativeConfigure(engine, fuzzy, typo);
      _last_fuzzy = fuzzy;
      _last_typo_correction = typo;
      _configured = true;
    }
  }

  private static void set_context(String context)
  {
    long engine = _engine;
    if (engine == 0) return;
    String value = context == null ? "" : context;
    synchronized (WowtypePinyinEngine.class)
    {
      configure(engine);
      if (!value.equals(_last_context))
      {
        nativeSetContext(engine, value);
        _last_context = value;
      }
    }
  }

  private static boolean containsHan(String text)
  {
    for (int i = 0; i < text.length();)
    {
      int codepoint = text.codePointAt(i);
      i += Character.charCount(codepoint);
      Character.UnicodeBlock block = Character.UnicodeBlock.of(codepoint);
      if ((codepoint >= 0x20000 && codepoint <= 0x323AF)
          || block == Character.UnicodeBlock.CJK_UNIFIED_IDEOGRAPHS
          || block == Character.UnicodeBlock.CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A
          || block == Character.UnicodeBlock.CJK_COMPATIBILITY_IDEOGRAPHS)
        return true;
    }
    return false;
  }

  private static native long nativeNew(String lexicon, String model, String history, String userDict);
  private static native String[] nativeInput(long engine, String code, int limit);
  private static native void nativeConfigure(long engine, boolean fuzzy, boolean typoCorrection);
  private static native void nativeSetContext(long engine, String context);
  private static native void nativeRecordSelection(long engine, String word,
      String pinyin, String history, String userDict);
  private static native void nativeFree(long engine);

  private static final class NativeEngine implements Pinyin.Engine
  {
    @Override public List<Composer.Candidate> candidates(String input, String context)
    {
      return WowtypePinyinEngine.candidates(input, context, 1000);
    }

    @Override public void record_selection(String input, String word, String context)
    {
      long engine = _engine;
      if (engine != 0)
        synchronized (WowtypePinyinEngine.class)
        {
          set_context(context);
          nativeRecordSelection(engine, word, input, _history_path, _user_dict_path);
        }
    }
  }
}
