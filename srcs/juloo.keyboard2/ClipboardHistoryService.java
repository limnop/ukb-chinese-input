package juloo.keyboard2;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build.VERSION;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.concurrent.TimeUnit;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

public final class ClipboardHistoryService
{
  /** Start the service on startup and start listening to clipboard changes. */
  public static void on_startup(Context ctx, ClipboardPasteCallback cb)
  {
    get_service(ctx);
    _paste_callback = cb;
  }

  /** Start the service if it hasn't been started before. Returns [null] if the
      feature is unsupported. */
  public static ClipboardHistoryService get_service(Context ctx)
  {
    if (VERSION.SDK_INT <= 11)
      return null;
    if (_service == null)
      _service = new ClipboardHistoryService(ctx);
    return _service;
  }

  public static void set_history_enabled(boolean e)
  {
    Config.globalConfig().set_clipboard_history_enabled(e);
    if (_service == null)
      return;
    if (e)
      _service.add_current_clip();
    else
      _service.clear_history();
  }

  /** Send the given string to the editor. */
  public static void paste(String clip)
  {
    if (_paste_callback != null)
      _paste_callback.paste_from_clipboard_pane(clip);
  }

  /** Text currently copied to Android's system clipboard, if any. */
  public static String get_current_clip_text(Context ctx)
  {
    ClipData clip = get_current_clip_data(ctx);
    if (clip == null)
      return null;
    for (int i = 0; i < clip.getItemCount(); i++)
    {
      CharSequence text = clip.getItemAt(i).getText();
      if (text != null)
        return text.toString();
    }
    return null;
  }

  public static ClipData get_current_clip_data(Context ctx)
  {
    ClipboardHistoryService service = get_service(ctx);
    return service == null ? null : service.current_clip_data();
  }

  /** Replace Android's system clipboard with remotely downloaded text. */
  public static void set_current_clip_text(Context ctx, String text)
  {
    ClipboardHistoryService service = get_service(ctx);
    if (service != null)
      service.set_clip_text(text);
  }

  /** Replace Android's system clipboard with a downloaded file URI. */
  public static void set_current_clip_uri(Context ctx, Uri uri, String label)
  {
    ClipboardHistoryService service = get_service(ctx);
    if (service != null)
      service.set_clip_uri(uri, label);
  }

  /** The maximum size limits the amount of user data stored in memory but also
      gives a sense to the user that the history is not persisted and can be
      forgotten as soon as the app stops. */
  public static final int MAX_HISTORY_SIZE = 70;
  private static final String HISTORY_PREFS = "clipboard_history";
  private static final String HISTORY_KEY = "recent_items";

  static ClipboardHistoryService _service = null;
  static ClipboardPasteCallback _paste_callback = null;

  ClipboardManager _cm;
  ContentResolver _resolver;
  SharedPreferences _history_prefs;
  List<HistoryEntry> _history;
  OnClipboardHistoryChange _listener = null;

  ClipboardHistoryService(Context ctx)
  {
    _history = new ArrayList<HistoryEntry>();
    _cm = (ClipboardManager)ctx.getSystemService(Context.CLIPBOARD_SERVICE);
    _resolver = ctx.getContentResolver();
    try
    {
      _history_prefs = ctx.getSharedPreferences(HISTORY_PREFS,
          Context.MODE_PRIVATE);
      load_history();
    }
    catch (Exception _e) {}
    _cm.addPrimaryClipChangedListener(this.new SystemListener());
    add_current_clip();
  }

  void load_history()
  {
    if (_history_prefs == null)
      return;
    String stored = _history_prefs.getString(HISTORY_KEY, null);
    if (stored == null)
      return;
    try
    {
      JSONArray entries = new JSONArray(stored);
      long now = System.currentTimeMillis();
      for (int i = 0; i < entries.length() && _history.size() < MAX_HISTORY_SIZE;
          i++)
      {
        JSONObject entry = entries.getJSONObject(i);
        String content = entry.getString("content");
        long expiry = entry.getLong("expiry");
        if (!content.isEmpty() && expiry > now && !contains_history(content))
          _history.add(new HistoryEntry(content, expiry));
      }
      persist_history();
    }
    catch (JSONException _e)
    {
      _history.clear();
      persist_history();
    }
  }

  boolean contains_history(String content)
  {
    for (HistoryEntry entry : _history)
      if (entry.content.equals(content))
        return true;
    return false;
  }

  void persist_history()
  {
    if (_history_prefs == null)
      return;
    JSONArray entries = new JSONArray();
    try
    {
      for (HistoryEntry entry : _history)
      {
        JSONObject stored = new JSONObject();
        stored.put("content", entry.content);
        stored.put("expiry", entry.expiry_timestamp);
        entries.put(stored);
      }
      _history_prefs.edit().putString(HISTORY_KEY, entries.toString()).apply();
    }
    catch (JSONException _e) {}
  }

  public synchronized List<String> clear_expired_and_get_history()
  {
    long now_ms = System.currentTimeMillis();
    int old_size = _history.size();
    List<String> dst = new ArrayList<String>();
    Iterator<HistoryEntry> it = _history.iterator();
    while (it.hasNext())
    {
      HistoryEntry ent = it.next();
      if (ent.expiry_timestamp <= now_ms)
        it.remove();
      else
        dst.add(ent.content);
    }
    if (_history.size() != old_size)
      persist_history();
    return dst;
  }

  /** This will call [on_clipboard_history_change]. */
  public synchronized void remove_history_entry(String clip)
  {
    int last_pos = _history.size() - 1;
    boolean last_pos_changed = false;
    for (int pos = last_pos; pos >= 0; pos--)
    {
      if (!_history.get(pos).content.equals(clip))
        continue;
      // Removing the current clipboard, clear the system clipboard.
      if (pos == last_pos)
        last_pos_changed = true;
      _history.remove(pos);
    }
    if (last_pos_changed)
    {
      if (VERSION.SDK_INT >= 28)
        _cm.clearPrimaryClip();
      else
        _cm.setText("");
    }
    persist_history();
    if (_listener != null)
      _listener.on_clipboard_history_change();
  }

  /** Add clipboard entries to the history, skipping consecutive duplicates and
      empty strings. */
  public synchronized void add_clip(String clip)
  {
    if (!Config.globalConfig().clipboard_history_enabled)
      return;
    if (clip.equals("") || (_history.size() > 0
          && _history.get(0).content.equals(clip)))
      return;
    for (Iterator<HistoryEntry> it = _history.iterator(); it.hasNext(); )
      if (it.next().content.equals(clip))
        it.remove();
    while (_history.size() >= MAX_HISTORY_SIZE)
      _history.remove(_history.size() - 1);
    _history.add(0,new HistoryEntry(clip));
    persist_history();
    if (_listener != null)
      _listener.on_clipboard_history_change();
  }

  public synchronized void clear_history()
  {
    _history.clear();
    persist_history();
    if (_listener != null)
      _listener.on_clipboard_history_change();
  }

  public void set_on_clipboard_history_change(OnClipboardHistoryChange l) { _listener = l; }

  public static interface OnClipboardHistoryChange
  {
    public void on_clipboard_history_change();
  }

  /** Add what is currently in the system clipboard into the history. */
  void add_current_clip()
  {
    ClipData clip = null;
    // getPrimaryClip might throw when the keyboard is disconnected.
    try { clip = _cm.getPrimaryClip(); } catch (Exception _e) {}
    if (clip == null)
      return;
    int count = clip.getItemCount();
    for (int i = 0; i < count; i++)
    {
      CharSequence text = clip.getItemAt(i).getText();
      if (text != null)
        add_clip(text.toString());
    }
  }

  synchronized String current_clip_text()
  {
    ClipData clip = current_clip_data();
    if (clip == null || clip.getItemCount() == 0)
      return null;
    CharSequence text = clip.getItemAt(0).getText();
    return text == null ? null : text.toString();
  }

  synchronized ClipData current_clip_data()
  {
    try { return _cm.getPrimaryClip(); }
    catch (Exception _e) { return null; }
  }

  synchronized void set_clip_text(String text)
  {
    if (text.equals(current_clip_text()))
    {
      add_clip(text);
      return;
    }
    _cm.setPrimaryClip(ClipData.newPlainText("SyncClipboard", text));
    add_clip(text);
  }

  synchronized void set_clip_uri(Uri uri, String label)
  {
    ClipData current = current_clip_data();
    if (current != null && current.getItemCount() > 0
        && uri.equals(current.getItemAt(0).getUri()))
      return;
    _cm.setPrimaryClip(ClipData.newUri(_resolver, label, uri));
  }

  final class SystemListener implements ClipboardManager.OnPrimaryClipChangedListener
  {
    public SystemListener() {}

    @Override
    public void onPrimaryClipChanged()
    {
      add_current_clip();
    }
  }

  static final class HistoryEntry
  {
    public final String content;

    /** Time at which the entry expires. */
    public final long expiry_timestamp;

    public HistoryEntry(String c)
    {
      content = c;
      final int historyTtlMinutes =
          Config.globalConfig().clipboard_history_duration;
      if (historyTtlMinutes >= 0)
      {
        expiry_timestamp = System.currentTimeMillis()
            + TimeUnit.MINUTES.toMillis(historyTtlMinutes);
      }
      else
        expiry_timestamp = Long.MAX_VALUE;
    }

    public HistoryEntry(String c, long expiry)
    {
      content = c;
      expiry_timestamp = expiry;
    }
  }

  public interface ClipboardPasteCallback
  {
    public void paste_from_clipboard_pane(String content);
  }
}
