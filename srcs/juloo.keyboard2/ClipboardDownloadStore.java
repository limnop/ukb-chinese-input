package juloo.keyboard2;

import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build.VERSION;
import android.os.Environment;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;

/** Saves manually downloaded SyncClipboard attachments in Downloads/UKB-Sync. */
final class ClipboardDownloadStore
{
  private static final String RELATIVE_PATH = ClipboardSyncClient.DOWNLOAD_DIR;

  static ClipboardSyncClient.SavedFile save(Context context, File source,
      String original_name, String mime, String content_hash) throws Exception
  {
    Uri existing = find_uri(context, stored_name(original_name, content_hash));
    if (existing != null)
      return new ClipboardSyncClient.SavedFile(existing, original_name, mime,
          source.length());

    if (VERSION.SDK_INT >= 29)
      return save_media_store(context, source, original_name, mime, content_hash);
    return save_app_external(context, source, original_name, mime, content_hash);
  }

  static ClipboardSyncClient.SavedFile find(Context context,
      String original_name, String content_hash)
  {
    Uri uri = find_uri(context, stored_name(original_name, content_hash));
    return uri == null ? null : new ClipboardSyncClient.SavedFile(uri,
        original_name, ClipboardSyncClient.URLConnectionGuess.type(original_name),
        size(context, uri));
  }

  private static ClipboardSyncClient.SavedFile save_media_store(Context context,
      File source, String original_name, String mime, String content_hash)
      throws Exception
  {
    ContentResolver resolver = context.getContentResolver();
    ContentValues values = new ContentValues();
    values.put(MediaStore.MediaColumns.DISPLAY_NAME,
        stored_name(original_name, content_hash));
    values.put(MediaStore.MediaColumns.MIME_TYPE, mime);
    values.put(MediaStore.MediaColumns.RELATIVE_PATH, RELATIVE_PATH);
    values.put(MediaStore.MediaColumns.IS_PENDING, 1);
    Uri collection = MediaStore.Downloads.getContentUri(
        MediaStore.VOLUME_EXTERNAL_PRIMARY);
    Uri target = resolver.insert(collection, values);
    if (target == null)
      throw new Exception("无法创建手机下载文件");
    try
    {
      OutputStream out = resolver.openOutputStream(target, "w");
      if (out == null)
        throw new Exception("无法写入手机下载文件");
      copy(source, out);
      ContentValues ready = new ContentValues();
      ready.put(MediaStore.MediaColumns.IS_PENDING, 0);
      resolver.update(target, ready, null, null);
      return new ClipboardSyncClient.SavedFile(target, original_name, mime,
          source.length());
    }
    catch (Exception e)
    {
      resolver.delete(target, null, null);
      throw e;
    }
  }

  private static ClipboardSyncClient.SavedFile save_app_external(Context context,
      File source, String original_name, String mime, String content_hash)
      throws Exception
  {
    File root = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS);
    if (root == null)
      throw new Exception("手机没有可用的外部存储空间");
    File dir = new File(root, "UKB-Sync");
    if (!dir.exists() && !dir.mkdirs())
      throw new Exception("无法创建剪贴板同步目录");
    File target = new File(dir, stored_name(original_name, content_hash));
    if (!target.exists())
      copy(source, target);
    return new ClipboardSyncClient.SavedFile(Uri.fromFile(target), original_name,
        mime, target.length());
  }

  private static Uri find_uri(Context context, String name)
  {
    if (VERSION.SDK_INT >= 29)
    {
      Uri collection = MediaStore.Downloads.getContentUri(
          MediaStore.VOLUME_EXTERNAL_PRIMARY);
      Cursor cursor = null;
      try
      {
        cursor = context.getContentResolver().query(collection,
            new String[]{ MediaStore.MediaColumns._ID },
            MediaStore.MediaColumns.DISPLAY_NAME + "=? AND "
              + MediaStore.MediaColumns.RELATIVE_PATH + "=?",
            new String[]{ name, RELATIVE_PATH }, null);
        if (cursor != null && cursor.moveToFirst())
          return ContentUris.withAppendedId(collection,
              cursor.getLong(0));
      }
      catch (Exception _e) {}
      finally { if (cursor != null) cursor.close(); }
      return null;
    }

    File root = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS);
    File file = root == null ? null : new File(new File(root, "UKB-Sync"), name);
    return file != null && file.exists() ? Uri.fromFile(file) : null;
  }

  private static long size(Context context, Uri uri)
  {
    Cursor cursor = null;
    try
    {
      cursor = context.getContentResolver().query(uri,
          new String[]{ OpenableColumns.SIZE }, null, null, null);
      if (cursor != null && cursor.moveToFirst())
        return cursor.getLong(0);
    }
    catch (Exception _e) {}
    finally { if (cursor != null) cursor.close(); }
    return 0;
  }

  private static String stored_name(String original_name, String hash)
  {
    String id = (hash == null || hash.length() < 16)
      ? "unknown" : hash.substring(0, Math.min(64, hash.length()));
    /* Content identity, rather than the source filename, makes repeats
       idempotent even when two devices name the same bytes differently. The
       MIME type is stored separately in MediaStore and still opens correctly. */
    return "UKB-Sync-" + id;
  }

  private static void copy(File source, File target) throws Exception
  {
    copy(source, new java.io.FileOutputStream(target));
  }

  private static void copy(File source, OutputStream out) throws Exception
  {
    InputStream in = new FileInputStream(source);
    byte[] buffer = new byte[32768];
    try
    {
      int n;
      while ((n = in.read(buffer)) >= 0)
        out.write(buffer, 0, n);
      out.flush();
    }
    finally
    {
      try { in.close(); } catch (Exception _e) {}
      try { out.close(); } catch (Exception _e) {}
    }
  }
}
