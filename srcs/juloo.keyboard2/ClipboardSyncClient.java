package juloo.keyboard2;

import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Context;
import android.content.SharedPreferences;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.provider.OpenableColumns;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.security.cert.CertificateException;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.net.HttpURLConnection;
import java.net.URI;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;
import java.util.TimeZone;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import javax.net.ssl.HttpsURLConnection;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLSocketFactory;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;
import org.json.JSONObject;

/** Lightweight client for SyncClipboard's documented WebDAV-compatible
    profile API: https://github.com/Jeric-X/SyncClipboard#api . */
final class ClipboardSyncClient
{
  static final String PREFS = "syncclipboard_config";
  static final String PREF_URL = "url";
  static final String PREF_USER = "user";
  static final String PREF_PASSWORD = "password";
  static final String SETTING_URL = "syncclipboard_url";
  static final String SETTING_USER = "syncclipboard_user";
  static final String SETTING_PASSWORD = "syncclipboard_password";
  static final String PREF_LEGACY_SETTINGS_MIGRATED = "settings_migrated_to_ukb";
  private static final String PINNED_SERVER_HOST = "123.249.80.39";
  static final String PREF_UP_TEXT_HASHES = "uploaded_text_hashes";
  static final String PREF_DOWN_TEXT_HASHES = "downloaded_text_hashes";
  static final String PREF_UP_FILE_HASHES = "uploaded_file_hashes";
  static final String PREF_DOWN_FILE_HASHES = "downloaded_file_hashes";
  private static final int HASH_HISTORY_LIMIT = 128;
  static final String DOWNLOAD_DIR = "Download/UKB-Sync/";
  private static final int TIMEOUT_MS = 15000;
  private static final int MAX_TEXT_BYTES = 4 * 1024 * 1024;
  private static final long MAX_FILE_BYTES = 100L * 1024 * 1024;
  private static final int TEXT_TRANSFER_THRESHOLD = 10240;
  private static final ExecutorService EXECUTOR =
    Executors.newSingleThreadExecutor();
  private static final Handler MAIN = new Handler(Looper.getMainLooper());

  interface Callback
  {
    void complete(Result result);
  }

  static final class Account
  {
    final String url, user, password;
    final SharedPreferences prefs;
    final Context context;

    Account(Context context)
    {
      this.context = context.getApplicationContext();
      prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
      SharedPreferences settings = DirectBootAwarePreferences
          .get_shared_preferences(context);
      url = settings.getString(SETTING_URL, "").trim();
      user = settings.getString(SETTING_USER, "");
      password = settings.getString(SETTING_PASSWORD, "");
    }

    boolean configured() { return url.length() > 0; }
  }

  static final class Result
  {
    final String message;
    final String text;
    final SavedFile file;
    final Bitmap image_preview;
    final String text_preview;
    final boolean duplicate;

    Result(String m, String t, SavedFile f, Bitmap image, String preview,
        boolean dup)
    {
      message = m;
      text = t;
      file = f;
      image_preview = image;
      text_preview = preview;
      duplicate = dup;
    }
  }

  static final class SavedFile
  {
    final Uri uri;
    final String name;
    final String mime;
    final long size;

    SavedFile(Uri u, String n, String m, long s)
    {
      uri = u;
      name = n;
      mime = m;
      size = s;
    }
  }

  static void push(Context context, final ClipData clip,
      final Callback callback)
  {
    final Context app = context.getApplicationContext();
    final Account account = new Account(app);
    EXECUTOR.execute(new Runnable()
        {
          @Override public void run()
          {
            Result result;
            try { result = push_clipboard(app, account, clip); }
            catch (Exception e) { result = failure(e); }
            post(callback, result);
          }
        });
  }

  static void pull(Context context, final Callback callback)
  {
    final Context app = context.getApplicationContext();
    final Account account = new Account(app);
    EXECUTOR.execute(new Runnable()
        {
          @Override public void run()
          {
            Result result;
            try { result = pull_profile(app, account); }
            catch (Exception e) { result = failure(e); }
            post(callback, result);
          }
        });
  }

  private static void post(final Callback callback, final Result result)
  {
    MAIN.post(new Runnable()
        {
          @Override public void run() { callback.complete(result); }
        });
  }

  private static Result push_clipboard(Context context, Account account,
      ClipData clip) throws Exception
  {
    if (clip == null || clip.getItemCount() == 0)
      throw new Exception("系统剪贴板为空");
    ClipData.Item item = clip.getItemAt(0);
    Uri uri = item.getUri();
    if (uri != null)
      return push_file(context, account, uri);
    CharSequence value = item.getText();
    if (value == null)
      throw new Exception("当前剪贴板内容无法同步");
    return push_text(account, value.toString());
  }

  private static Result push_text(Account account, String text)
      throws Exception
  {
    byte[] bytes = text.getBytes(StandardCharsets.UTF_8);
    String hash = sha256(bytes);
    if (has_hash(account, PREF_DOWN_TEXT_HASHES, hash)
        || has_hash(account, PREF_UP_TEXT_HASHES, hash))
      return result("文本内容未变化，已跳过重复同步", null, null, null, null, true);

    String preview = text;
    boolean has_data = text.length() > TEXT_TRANSFER_THRESHOLD;
    String data_name = null;
    if (has_data)
    {
      if (bytes.length > MAX_TEXT_BYTES)
        throw new Exception("文本超过 4 MB 限制");
      data_name = "Text_" + hash.substring(0, 24) + ".txt";
      put_file(account, data_name, bytes);
      preview = text.substring(0, Math.min(text.length(),
          TEXT_TRANSFER_THRESHOLD));
    }

    JSONObject profile = new JSONObject();
    profile.put("type", "Text");
    profile.put("hash", hash);
    profile.put("text", preview);
    profile.put("hasData", has_data);
    if (has_data)
    {
      profile.put("dataName", data_name);
      profile.put("transferDataHash", hash);
    }
    profile.put("size", text.length());
    profile.put("syncedAt", synced_at());
    put_profile(account, profile);
    remember_hash(account, PREF_UP_TEXT_HASHES, hash);
    return result("文本上传完成", null, null, null, null, false);
  }

  private static Result push_file(Context context, Account account, Uri uri)
      throws Exception
  {
    ContentResolver resolver = context.getContentResolver();
    String name = safe_name(display_name(resolver, uri));
    String mime = resolver.getType(uri);
    if (mime == null || mime.length() == 0)
      mime = URLConnectionGuess.type(name);
    String type = mime.startsWith("image/") ? "Image" : "File";
    File temp = File.createTempFile("ukb-sync-up-", ".bin",
        context.getCacheDir());
    String transfer_hash;
    try
    {
      InputStream in = resolver.openInputStream(uri);
      if (in == null)
        throw new Exception("无法读取剪贴板文件");
      copy_bounded(in, new FileOutputStream(temp), MAX_FILE_BYTES);
      transfer_hash = sha256(temp);
      if (has_hash(account, PREF_DOWN_FILE_HASHES, transfer_hash)
          || has_hash(account, PREF_UP_FILE_HASHES, transfer_hash))
        return result("文件内容未变化，已跳过重复同步", null, null, null,
            null, true);
      put_file(account, name, temp);
      String profile_hash = sha256((name + "|" + transfer_hash)
          .getBytes(StandardCharsets.UTF_8));
      JSONObject profile = new JSONObject();
      profile.put("type", type);
      profile.put("hash", profile_hash);
      profile.put("text", name);
      profile.put("hasData", true);
      profile.put("dataName", name);
      profile.put("transferDataHash", transfer_hash);
      profile.put("size", temp.length());
      profile.put("syncedAt", synced_at());
      put_profile(account, profile);
      remember_hash(account, PREF_UP_FILE_HASHES, transfer_hash);
      return result("文件上传完成：" + name, null, null, null, null, false);
    }
    finally { temp.delete(); }
  }

  private static Result pull_profile(Context context, Account account)
      throws Exception
  {
    JSONObject profile = get_profile(account);
    String type = profile.optString("type", "");
    if ("Text".equals(type))
      return pull_text(account, profile);
    if (!"File".equals(type) && !"Image".equals(type) && !"Group".equals(type))
      throw new Exception("远端剪贴板类型暂不支持：" + type);
    if (!profile.optBoolean("hasData", false))
      throw new Exception("远端文件数据不完整");

    String data_name = safe_name(profile.optString("dataName", ""));
    if (data_name.length() == 0)
      throw new Exception("远端文件名称无效");
    String announced_hash = profile.optString("transferDataHash", "");
    String mime = "Group".equals(type) ? "application/zip"
        : URLConnectionGuess.type(data_name);
    if ("Image".equals(type) && !mime.startsWith("image/"))
      mime = "image/png";
    if (announced_hash.length() > 0
        && has_hash(account, PREF_UP_FILE_HASHES, announced_hash))
      return result("远端文件是本机已上传的内容，已跳过重复下载",
          null, null, null, null, true);
    if (announced_hash.length() > 0
        && has_hash(account, PREF_DOWN_FILE_HASHES, announced_hash))
    {
      SavedFile existing = ClipboardDownloadStore.find(context, data_name,
          announced_hash);
      if (existing != null)
      {
        Bitmap image = mime.startsWith("image/")
            ? decode_preview(context, existing.uri) : null;
        String preview = is_text_file(data_name, mime)
            ? read_text_preview(context, existing.uri) : null;
        return result("文件已同步，未重复下载", null, existing, image, preview,
            true);
      }
    }
    File temp = File.createTempFile("ukb-sync-down-", ".bin",
        context.getCacheDir());
    try
    {
      URL data_url = endpoint(account, "file/" + encode_path_segment(data_name));
      HttpURLConnection conn = open(account, data_url, "GET");
      try
      {
        require_success(conn);
        copy_bounded(conn.getInputStream(), new FileOutputStream(temp),
            MAX_FILE_BYTES);
      }
      finally { conn.disconnect(); }

      String transfer_hash = sha256(temp);
      String expected_data_hash = profile.optString("transferDataHash", "");
      if (expected_data_hash.length() > 0
          && !expected_data_hash.equalsIgnoreCase(transfer_hash))
        throw new Exception("远端文件校验失败");
      if (!"Group".equals(type))
      {
        String expected_profile_hash = profile.optString("hash", "");
        String actual_profile_hash = sha256((data_name + "|" + transfer_hash)
            .getBytes(StandardCharsets.UTF_8));
        if (expected_profile_hash.length() > 0
            && !expected_profile_hash.equalsIgnoreCase(actual_profile_hash))
          throw new Exception("远端文件名称或内容校验失败");
      }

      if (has_hash(account, PREF_DOWN_FILE_HASHES, transfer_hash))
      {
        SavedFile existing = ClipboardDownloadStore.find(context, data_name,
            transfer_hash);
        if (existing != null)
          return result("文件已同步，未重复保存", null, existing, null, null, true);
      }

      SavedFile saved = ClipboardDownloadStore.save(context, temp, data_name,
          mime, transfer_hash);
      remember_hash(account, PREF_DOWN_FILE_HASHES, transfer_hash);
      Bitmap image = mime.startsWith("image/")
          ? decode_preview(context, saved.uri) : null;
      String preview = is_text_file(data_name, mime)
          ? read_text_preview(context, saved.uri) : null;
      return result("文件已下载：" + data_name, null, saved, image, preview, false);
    }
    finally { temp.delete(); }
  }

  private static Result pull_text(Account account, JSONObject profile)
      throws Exception
  {
    String text;
    if (profile.optBoolean("hasData", false))
    {
      String data_name = safe_name(profile.optString("dataName", ""));
      if (data_name.length() == 0)
        throw new Exception("远端文本附件名称无效");
      URL url = endpoint(account, "file/" + encode_path_segment(data_name));
      HttpURLConnection conn = open(account, url, "GET");
      byte[] bytes;
      try
      {
        require_success(conn);
        bytes = read_bounded(conn.getInputStream(), MAX_TEXT_BYTES);
      }
      finally { conn.disconnect(); }
      String data_hash = profile.optString("transferDataHash", "");
      if (data_hash.length() > 0 && !data_hash.equalsIgnoreCase(sha256(bytes)))
        throw new Exception("远端文本附件校验失败");
      text = new String(bytes, StandardCharsets.UTF_8);
    }
    else
      text = profile.optString("text", "");

    String hash = sha256(text.getBytes(StandardCharsets.UTF_8));
    String expected = profile.optString("hash", "");
    if (expected.length() > 0 && !expected.equalsIgnoreCase(hash))
      throw new Exception("远端文本校验失败");
    if (has_hash(account, PREF_DOWN_TEXT_HASHES, hash)
        || has_hash(account, PREF_UP_TEXT_HASHES, hash))
      return result("文本已同步，未重复写入", text, null, null, null, true);
    remember_hash(account, PREF_DOWN_TEXT_HASHES, hash);
    return result("文本下载完成", text, null, null, null, false);
  }

  private static JSONObject get_profile(Account account) throws Exception
  {
    HttpURLConnection conn = open(account,
        endpoint(account, "SyncClipboard.json"), "GET");
    try
    {
      require_success(conn);
      return new JSONObject(new String(read_bounded(conn.getInputStream(),
              1024 * 1024), StandardCharsets.UTF_8));
    }
    finally { conn.disconnect(); }
  }

  private static void put_profile(Account account, JSONObject profile)
      throws Exception
  {
    byte[] bytes = profile.toString().getBytes(StandardCharsets.UTF_8);
    HttpURLConnection conn = open(account,
        endpoint(account, "SyncClipboard.json"), "PUT");
    try
    {
      conn.setDoOutput(true);
      conn.setRequestProperty("Content-Type", "application/json; charset=utf-8");
      conn.setFixedLengthStreamingMode(bytes.length);
      OutputStream out = conn.getOutputStream();
      try { out.write(bytes); }
      finally { out.close(); }
      require_success(conn);
    }
    finally { conn.disconnect(); }
  }

  private static void put_file(Account account, String name, File file)
      throws Exception
  {
    HttpURLConnection conn = open(account, endpoint(account,
          "file/" + encode_path_segment(name)), "PUT");
    try
    {
      conn.setDoOutput(true);
      conn.setRequestProperty("Content-Type", "application/octet-stream");
      conn.setFixedLengthStreamingMode(file.length());
      InputStream in = new FileInputStream(file);
      OutputStream out = conn.getOutputStream();
      copy_bounded(in, out, MAX_FILE_BYTES);
      require_success(conn);
    }
    finally { conn.disconnect(); }
  }

  private static void put_file(Account account, String name, byte[] bytes)
      throws Exception
  {
    HttpURLConnection conn = open(account, endpoint(account,
          "file/" + encode_path_segment(name)), "PUT");
    try
    {
      conn.setDoOutput(true);
      conn.setRequestProperty("Content-Type", "text/plain; charset=utf-8");
      conn.setFixedLengthStreamingMode(bytes.length);
      OutputStream out = conn.getOutputStream();
      try { out.write(bytes); }
      finally { out.close(); }
      require_success(conn);
    }
    finally { conn.disconnect(); }
  }

  private static URL endpoint(Account account, String path) throws Exception
  {
    if (!account.configured())
      throw new Exception("请先设置 SyncClipboard 服务地址");
    URI uri = new URI(account.url);
    if (!"https".equalsIgnoreCase(uri.getScheme()) || uri.getHost() == null
        || uri.getRawQuery() != null || uri.getRawFragment() != null)
      throw new Exception("服务地址必须是 HTTPS，且不能带查询参数");
    String base = account.url;
    while (base.endsWith("/"))
      base = base.substring(0, base.length() - 1);
    return new URL(base + "/" + path);
  }

  private static HttpURLConnection open(Account account, URL url,
      String method) throws Exception
  {
    HttpURLConnection conn = (HttpURLConnection)url.openConnection();
    if (conn instanceof HttpsURLConnection
        && PINNED_SERVER_HOST.equalsIgnoreCase(url.getHost()))
      ((HttpsURLConnection)conn).setSSLSocketFactory(
          pinned_server_socket_factory(account.context));
    conn.setRequestMethod(method);
    conn.setConnectTimeout(TIMEOUT_MS);
    conn.setReadTimeout(TIMEOUT_MS);
    conn.setInstanceFollowRedirects(false);
    String auth = account.user + ":" + account.password;
    String encoded = android.util.Base64.encodeToString(
        auth.getBytes(StandardCharsets.UTF_8), android.util.Base64.NO_WRAP);
    conn.setRequestProperty("Authorization", "Basic " + encoded);
    conn.setRequestProperty("Accept", "application/json");
    return conn;
  }

  private static SSLSocketFactory pinned_server_socket_factory(Context context)
      throws Exception
  {
    final X509Certificate pinned;
    java.io.InputStream cert_stream = context.getResources()
        .openRawResource(R.raw.syncclipboard_server_ca);
    try
    {
      pinned = (X509Certificate)CertificateFactory.getInstance("X.509")
          .generateCertificate(cert_stream);
    }
    finally { cert_stream.close(); }
    pinned.checkValidity();

    X509TrustManager trust_manager = new X509TrustManager()
    {
      @Override public void checkClientTrusted(X509Certificate[] chain,
          String auth_type) throws CertificateException
      {
        throw new CertificateException("Client certificates are not accepted");
      }

      @Override public void checkServerTrusted(X509Certificate[] chain,
          String auth_type) throws CertificateException
      {
        if (chain == null || chain.length == 0 || !pinned.equals(chain[0]))
          throw new CertificateException("Unexpected SyncClipboard server certificate");
        chain[0].checkValidity();
      }

      @Override public X509Certificate[] getAcceptedIssuers()
      {
        return new X509Certificate[] { pinned };
      }
    };
    SSLContext ssl_context = SSLContext.getInstance("TLS");
    ssl_context.init(null, new TrustManager[] { trust_manager }, null);
    return ssl_context.getSocketFactory();
  }

  private static void require_success(HttpURLConnection conn) throws Exception
  {
    int status = conn.getResponseCode();
    if (status < 200 || status >= 300)
    {
      if (status == 401 || status == 403)
        throw new Exception("认证失败，请检查用户名和密码");
      throw new Exception("服务器返回 HTTP " + status);
    }
  }

  private static void copy_bounded(InputStream in, OutputStream out, long limit)
      throws Exception
  {
    byte[] buffer = new byte[32768];
    long total = 0;
    try
    {
      int n;
      while ((n = in.read(buffer)) >= 0)
      {
        total += n;
        if (total > limit)
          throw new Exception(limit <= MAX_TEXT_BYTES
              ? "远端文本超过 4 MB 限制" : "同步文件超过 100 MB 限制");
        out.write(buffer, 0, n);
      }
      out.flush();
    }
    finally
    {
      try { in.close(); } catch (Exception _e) {}
      try { out.close(); } catch (Exception _e) {}
    }
  }

  private static byte[] read_bounded(InputStream in, int limit) throws Exception
  {
    ByteArrayOutputStream out = new ByteArrayOutputStream();
    copy_bounded(in, out, limit);
    return out.toByteArray();
  }

  private static String display_name(ContentResolver resolver, Uri uri)
  {
    Cursor cursor = null;
    try
    {
      cursor = resolver.query(uri, new String[]{ OpenableColumns.DISPLAY_NAME },
          null, null, null);
      if (cursor != null && cursor.moveToFirst())
      {
        int i = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
        if (i >= 0)
          return cursor.getString(i);
      }
    }
    catch (Exception _e) {}
    finally { if (cursor != null) cursor.close(); }
    String path = uri.getLastPathSegment();
    return path == null ? "clipboard-file" : path;
  }

  static String safe_name(String name)
  {
    if (name == null)
      return "";
    name = name.replace('\\', '/');
    int slash = name.lastIndexOf('/');
    if (slash >= 0)
      name = name.substring(slash + 1);
    StringBuilder clean = new StringBuilder();
    for (int i = 0; i < name.length(); i++)
    {
      char c = name.charAt(i);
      if (c >= 32 && c != 127 && c != '/' && c != '\\')
        clean.append(c);
    }
    name = clean.toString().trim();
    if (name.equals(".") || name.equals(".."))
      return "";
    return name;
  }

  private static String encode_path_segment(String name) throws Exception
  {
    return java.net.URLEncoder.encode(name, "UTF-8").replace("+", "%20");
  }

  private static String synced_at()
  {
    SimpleDateFormat date = new SimpleDateFormat(
        "yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US);
    date.setTimeZone(TimeZone.getTimeZone("UTC"));
    return date.format(new Date());
  }

  private static boolean has_hash(Account account, String key, String hash)
  {
    Set<String> values = account.prefs.getStringSet(key,
        java.util.Collections.<String>emptySet());
    return values.contains(hash);
  }

  private static void remember_hash(Account account, String key, String hash)
  {
    Set<String> values = new HashSet<String>(account.prefs.getStringSet(key,
          java.util.Collections.<String>emptySet()));
    if (values.contains(hash))
      return;
    values.add(hash);
    while (values.size() > HASH_HISTORY_LIMIT)
    {
      String first = values.iterator().next();
      values.remove(first);
    }
    account.prefs.edit().putStringSet(key, values).apply();
  }

  private static String sha256(File file) throws Exception
  {
    MessageDigest digest = MessageDigest.getInstance("SHA-256");
    InputStream in = new FileInputStream(file);
    try
    {
      byte[] buffer = new byte[32768];
      int n;
      while ((n = in.read(buffer)) >= 0)
        digest.update(buffer, 0, n);
    }
    finally { in.close(); }
    return hex(digest.digest());
  }

  private static String sha256(byte[] data) throws Exception
  {
    return hex(MessageDigest.getInstance("SHA-256").digest(data));
  }

  private static String hex(byte[] digest)
  {
    final char[] digits = "0123456789ABCDEF".toCharArray();
    StringBuilder out = new StringBuilder(digest.length * 2);
    for (byte b : digest)
    {
      out.append(digits[(b >>> 4) & 0xf]);
      out.append(digits[b & 0xf]);
    }
    return out.toString();
  }

  private static Bitmap decode_preview(Context context, Uri uri)
  {
    InputStream in = null;
    try
    {
      BitmapFactory.Options bounds = new BitmapFactory.Options();
      bounds.inJustDecodeBounds = true;
      in = context.getContentResolver().openInputStream(uri);
      BitmapFactory.decodeStream(in, null, bounds);
      in.close();
      int sample = 1;
      while (bounds.outWidth / sample > 320 || bounds.outHeight / sample > 240)
        sample *= 2;
      BitmapFactory.Options options = new BitmapFactory.Options();
      options.inSampleSize = sample;
      in = context.getContentResolver().openInputStream(uri);
      return BitmapFactory.decodeStream(in, null, options);
    }
    catch (Exception _e) { return null; }
    finally { if (in != null) try { in.close(); } catch (Exception _e) {} }
  }

  private static boolean is_text_file(String name, String mime)
  {
    return (mime != null && mime.startsWith("text/"))
      || name.endsWith(".txt") || name.endsWith(".md")
      || name.endsWith(".json") || name.endsWith(".xml")
      || name.endsWith(".csv") || name.endsWith(".log");
  }

  private static String read_text_preview(Context context, Uri uri)
  {
    InputStream in = null;
    try
    {
      in = context.getContentResolver().openInputStream(uri);
      if (in == null)
        return null;
      byte[] data = new byte[2048];
      int n = in.read(data);
      if (n <= 0)
        return "(空文件)";
      String text = new String(data, 0, n, StandardCharsets.UTF_8);
      if (text.indexOf('\u0000') >= 0)
        return null;
      return text + (n == data.length ? "…" : "");
    }
    catch (Exception _e) { return null; }
    finally { if (in != null) try { in.close(); } catch (Exception _e) {} }
  }

  private static Result failure(Exception e)
  {
    String message = e.getMessage();
    return result((message == null || message.length() == 0)
        ? "同步失败" : "同步失败：" + message,
        null, null, null, null, false);
  }

  private static Result result(String message, String text, SavedFile file,
      Bitmap image, String preview, boolean duplicate)
  {
    return new Result(message, text, file, image, preview, duplicate);
  }

  /** Kept separate so filename inference can be substituted in tests. */
  static final class URLConnectionGuess
  {
    static String type(String name)
    {
      String lower = name.toLowerCase(Locale.US);
      if (lower.endsWith(".png")) return "image/png";
      if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
      if (lower.endsWith(".gif")) return "image/gif";
      if (lower.endsWith(".webp")) return "image/webp";
      if (lower.endsWith(".txt") || lower.endsWith(".log")) return "text/plain";
      if (lower.endsWith(".md")) return "text/markdown";
      if (lower.endsWith(".json")) return "application/json";
      if (lower.endsWith(".xml")) return "application/xml";
      if (lower.endsWith(".csv")) return "text/csv";
      if (lower.endsWith(".zip")) return "application/zip";
      return "application/octet-stream";
    }
  }
}
