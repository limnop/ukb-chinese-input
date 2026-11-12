package juloo.keyboard2;

import android.content.SharedPreferences;
import android.content.res.Configuration;
import android.os.Build;
import android.os.Bundle;
import android.preference.PreferenceActivity;
import android.preference.PreferenceManager;

public class SettingsActivity extends PreferenceActivity
{
  @Override
  public void onCreate(Bundle savedInstanceState)
  {
    super.onCreate(savedInstanceState);
    // The preferences can't be read when in direct-boot mode. Avoid crashing
    // and don't allow changing the settings.
    // Run the config migration on this prefs as it might be different from the
    // one used by the keyboard, which have been migrated.
    try
    {
      Config.migrate(getPreferenceManager().getSharedPreferences());
    }
    catch (Exception _e) { fallbackEncrypted(); return; }
    addPreferencesFromResource(R.xml.settings);
    migrate_syncclipboard_settings();

    boolean foldableDevice = FoldStateTracker.isFoldableDevice(this);
    findPreference("margin_bottom_portrait_unfolded").setEnabled(foldableDevice);
    findPreference("margin_bottom_landscape_unfolded").setEnabled(foldableDevice);
    findPreference("horizontal_margin_portrait_unfolded").setEnabled(foldableDevice);
    findPreference("horizontal_margin_landscape_unfolded").setEnabled(foldableDevice);
    findPreference("keyboard_height_unfolded").setEnabled(foldableDevice);
    findPreference("keyboard_height_landscape_unfolded").setEnabled(foldableDevice);
  }

  void migrate_syncclipboard_settings()
  {
    SharedPreferences settings = getPreferenceManager().getSharedPreferences();
    SharedPreferences old = getSharedPreferences(
        ClipboardSyncClient.PREFS, MODE_PRIVATE);
    if (old.getBoolean(ClipboardSyncClient.PREF_LEGACY_SETTINGS_MIGRATED,
          false))
      return;
    SharedPreferences.Editor edit = settings.edit();
    copy_legacy_sync_setting(settings, old, edit,
        ClipboardSyncClient.SETTING_URL, ClipboardSyncClient.PREF_URL);
    copy_legacy_sync_setting(settings, old, edit,
        ClipboardSyncClient.SETTING_USER, ClipboardSyncClient.PREF_USER);
    copy_legacy_sync_setting(settings, old, edit,
        ClipboardSyncClient.SETTING_PASSWORD, ClipboardSyncClient.PREF_PASSWORD);
    old.edit().putBoolean(ClipboardSyncClient.PREF_LEGACY_SETTINGS_MIGRATED,
        true).apply();
    edit.apply();
  }

  static void copy_legacy_sync_setting(SharedPreferences settings,
      SharedPreferences old, SharedPreferences.Editor edit, String key,
      String old_key)
  {
    String existing = settings.getString(key, "");
    String legacy = old.getString(old_key, "");
    if (existing.trim().isEmpty() && !legacy.trim().isEmpty())
      edit.putString(key, legacy);
  }

  void fallbackEncrypted()
  {
    // Can't communicate with the user here.
    finish();
  }

  protected void onStop()
  {
    DirectBootAwarePreferences
      .copy_preferences_to_protected_storage(this,
          getPreferenceManager().getSharedPreferences());
    super.onStop();
  }
}
