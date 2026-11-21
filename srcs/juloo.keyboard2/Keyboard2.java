package juloo.keyboard2;

import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.res.Configuration;
import android.graphics.drawable.Drawable;
import android.inputmethodservice.InputMethodService;
import android.os.Build.VERSION;
import android.os.Handler;
import android.os.IBinder;
import android.text.InputType;
import android.util.Log;
import android.util.LogPrinter;
import android.view.*;
import android.view.inputmethod.CursorAnchorInfo;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.view.inputmethod.InputMethodSubtype;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import java.util.AbstractMap.SimpleEntry;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import juloo.cdict.Cdict;
import juloo.keyboard2.staged.WowtypePinyinEngine;
import juloo.keyboard2.dict.Dictionaries;
import juloo.keyboard2.dict.DictionariesActivity;
import juloo.keyboard2.dict.DictionarySwitcher;
import juloo.keyboard2.prefs.LayoutsPreference;
import juloo.keyboard2.suggestions.CandidatesView;
import juloo.keyboard2.suggestions.FloatingCandidates;
import juloo.keyboard2.suggestions.Suggestions;
import juloo.keyboard2.staged.Composer;

public class Keyboard2 extends InputMethodService
  implements SharedPreferences.OnSharedPreferenceChangeListener
{
  private static final long CLIPBOARD_POLL_INTERVAL_MS = 10000;
  /** The view containing the keyboard and candidates view. */
  private ViewGroup _keyboard_container_view;
  private Keyboard2View _keyboard_layout_view;
  private CandidatesView _candidates_view;
  private FloatingCandidates _floating_candidates;
  private Suggestions _suggestions;
  private KeyEventHandler _keyeventhandler;
  /** If not 'null', the layout to use instead of [_config.current_layout]. */
  private KeyboardData _currentSpecialLayout;
  /** Layout associated with the currently selected locale. Not 'null'. */
  private KeyboardData _localeTextLayout;
  /** Installed and current locales. */
  private Dictionaries _dictionaries;
  private ViewGroup _emojiPane = null;
  private ViewGroup _clipboard_pane = null;
  private Handler _handler;
  private Runnable _clipboardPollTask;
  private boolean _clipboardPollInFlight = false;
  private int _clipboardPollGeneration = 0;
  private boolean _physical_chinese = true;
  /** Shift pressed by itself, pending release to switch languages. */
  private Long _physical_shift_down;
  /** Key-up and repeat events belong to whoever handled the initial down. */
  private final Set<Long> _physical_keys_down = new HashSet<>();

  private Config _config;

  private FoldStateTracker _foldStateTracker;

  /** Layout currently visible before it has been modified. */
  KeyboardData current_layout_unmodified()
  {
    if (_currentSpecialLayout != null)
      return _currentSpecialLayout;
    KeyboardData layout = null;
    int layout_i = _config.get_current_layout();
    if (layout_i >= _config.layouts.size())
      layout_i = 0;
    if (layout_i < _config.layouts.size())
      layout = _config.layouts.get(layout_i);
    if (layout == null)
      layout = _localeTextLayout;
    return layout;
  }

  /** Layout currently visible. */
  KeyboardData current_layout()
  {
    if (_currentSpecialLayout != null)
      return _currentSpecialLayout;
    return LayoutModifier.modify_layout(current_layout_unmodified());
  }

  void setTextLayout(int l)
  {
    _config.set_current_layout(l);
    _currentSpecialLayout = null;
    // The active dictionary depends on the current layout.
    refresh_current_dictionary();
    apply_current_layout();
  }

  void incrTextLayout(int delta)
  {
    int s = _config.layouts.size();
    setTextLayout((_config.get_current_layout() + delta + s) % s);
  }

  void setSpecialLayout(KeyboardData l)
  {
    _currentSpecialLayout = l;
    apply_current_layout();
  }

  private void apply_current_layout()
  {
    KeyboardData layout = current_layout();
    _keyboard_layout_view.setKeyboard(layout);
    String script = input_script(layout.script);
    _keyeventhandler.set_layout_script(script);
    refresh_candidates_view(script);
  }

  private String input_script(String layout_script)
  {
    if (!physical_input_active())
      return layout_script;
    return _physical_chinese ? "pinyin" : null;
  }

  KeyboardData loadLayout(int layout_id)
  {
    return KeyboardData.load(getResources(), layout_id);
  }

  /** Load a layout that contains a numpad. */
  KeyboardData loadNumpad(int layout_id)
  {
    return LayoutModifier.modify_numpad(KeyboardData.load(getResources(), layout_id),
        current_layout_unmodified());
  }

  KeyboardData loadNumericLayout()
  {
    return loadNumpad(_config.orientation_landscape ?
        R.xml.numeric_landscape : R.xml.numeric);
  }

  KeyboardData loadPinentry(int layout_id)
  {
    return LayoutModifier.modify_pinentry(KeyboardData.load(getResources(), layout_id),
        current_layout_unmodified());
  }

  @Override
  public void onCreate()
  {
    super.onCreate();
    SharedPreferences prefs = DirectBootAwarePreferences.get_shared_preferences(this);
    _handler = new Handler(getMainLooper());
    _foldStateTracker = new FoldStateTracker(this);
    _dictionaries = Dictionaries.instance(this);
    _dictionaries.install_bundled_zh();
    Config.initGlobalConfig(prefs, getResources(),
        _foldStateTracker.isUnfolded(), _dictionaries);
    _config = Config.globalConfig();
    Receiver recvr = this.new Receiver();
    _suggestions = new Suggestions(recvr, _config);
    _keyeventhandler = new KeyEventHandler(recvr, _suggestions);
    KeyValue.Stateful._handler = recvr;
    _config.handler = _keyeventhandler;
    prefs.registerOnSharedPreferenceChangeListener(this);
    Logs.set_debug_logs(getResources().getBoolean(R.bool.debug_logs));
    refreshSubtypeImm();
    create_keyboard_view();
    WowtypePinyinEngine.preload(this, () -> {
      if ((isInputViewShown() || physical_input_active())
          && _keyeventhandler.has_composition())
        _keyeventhandler.dictionary_changed();
    });
    ClipboardHistoryService.on_startup(this, _keyeventhandler);
    _foldStateTracker.setChangedCallback(() -> { refresh_config(); apply_current_layout(); });
  }

  @Override
  public void onDestroy() {
    stopClipboardPolling();
    super.onDestroy();

    _foldStateTracker.close();
  }

  private void create_keyboard_view()
  {
    _keyboard_container_view = (ViewGroup)inflate_view(R.layout.keyboard);
    _keyboard_layout_view = (Keyboard2View)_keyboard_container_view.findViewById(R.id.keyboard_view);
    _candidates_view = (CandidatesView)_keyboard_container_view.findViewById(R.id.candidates_view);
  }

  InputMethodManager get_imm()
  {
    return (InputMethodManager)getSystemService(INPUT_METHOD_SERVICE);
  }

  private void refreshSubtypeImm()
  {
    _config.shouldOfferVoiceTyping = true;
    KeyboardData default_layout = null;
    _config.device_locales = DeviceLocales.load(this);
    if (_config.device_locales.default_ != null)
    {
      String layout_name = _config.device_locales.default_.default_layout;
      if (layout_name != null)
        default_layout = LayoutsPreference.layout_of_string(getResources(), layout_name);
    }
    _config.extra_keys_subtype = _config.device_locales.extra_keys();
    if (default_layout == null)
      default_layout = loadLayout(R.xml.latn_qwerty_us);
    _localeTextLayout = default_layout;
  }

  private void refresh_current_dictionary()
  {
    _config.should_show_dictionary_switch =
      (_config.device_locales.installed.size() > 0);
    String dict_name = _dictionaries.get_selected(_config);
    /* Chinese layouts use the bundled zh dictionary regardless of the device
       locale: the locale default would pick an English word list. */
    if (dict_name == null && is_chinese_layout())
      dict_name = "zh";
    if (dict_name == null)
      dict_name = (_config.device_locales.default_ != null) ?
        _config.device_locales.default_.dictionary : null;
    _dictionaries.set_current_dictionary(_config, dict_name);
  }

  boolean is_chinese_layout()
  {
    int i = _config.get_current_layout();
    if (i < 0 || i >= _config.layouts.size())
      return false;
    KeyboardData layout = _config.layouts.get(i);
    return layout != null &&
      ("pinyin".equals(layout.script) || "zhuyin".equals(layout.script));
  }

  /** Remember and apply the dictionary chosen by the user for the current
      context. */
  private void select_dictionary(String dict_name)
  {
    _dictionaries.set_selected(_config, dict_name);
    refresh_current_dictionary();
    refresh_candidates_view(input_script(current_layout_unmodified().script));
  }

  private void refresh_candidates_view(String script)
  {
    boolean compact = physical_mode_selected();
    boolean should_show = compact ? physical_input_active()
      : _config.editor_config.show_candidate_row(script,
          _config.suggestions_enabled, _config.split_layout);
    _candidates_view.setVisibility(should_show ? View.VISIBLE : View.GONE);
    _candidates_view.refresh_config(_config, script, compact);
    _keyboard_container_view.requestApplyInsets();
    if (should_show)
      _keyeventhandler.dictionary_changed();
    update_floating_candidates();
  }

  /** Might re-create the keyboard view. [_keyboard_layout_view.setKeyboard()] and
      [setInputView()] must be called soon after. */
  private void refresh_config()
  {
    int prev_theme = _config.theme;
    _config.refresh(getResources(), _foldStateTracker.isUnfolded(), _dictionaries);
    refresh_current_dictionary();
    // Refreshing the theme config requires re-creating the views
    if (prev_theme != _config.theme)
    {
      create_keyboard_view();
      _emojiPane = null;
      _clipboard_pane = null;
      setInputView(_keyboard_container_view);
    }
    // Set keyboard background opacity
    if (_floating_candidates != null)
      _floating_candidates.set_opacity(_config.keyboardOpacity);
    else
    {
      Drawable bg = _keyboard_container_view.getBackground().mutate();
      bg.setAlpha(_config.keyboardOpacity);
      _keyboard_container_view.setBackground(bg);
    }
    _keyboard_layout_view.reset();
  }

  private KeyboardData refresh_special_layout()
  {
    if (_config.editor_config.numeric_layout)
    {
      switch (_config.selected_number_layout)
      {
        case PIN:
          return loadPinentry(_config.orientation_landscape ?
              R.xml.pin_landscape : R.xml.pin);
        case NUMBER:
          return loadNumericLayout();
      }
    }
    return null;
  }

  @Override
  public void onStartInput(EditorInfo info, boolean restarting)
  {
    super.onStartInput(info, restarting);
    _physical_shift_down = null;
    _config.editor_config.refresh(info, getResources());
    if (physical_mode_selected())
    {
      // The first physical keystroke can arrive before any IME view is shown.
      prepare_input();
      if (!physical_candidates_enabled() && isInputViewShown())
        requestHideSelf(0);
      else if (!restarting && physical_candidates_enabled() && !isInputViewShown())
        _handler.post(() -> {
          if (getCurrentInputEditorInfo() == info && physical_candidates_enabled() && !isInputViewShown())
            requestShowSelf(0);
        });
    }
  }

  private void prepare_input()
  {
    refresh_config();
    _currentSpecialLayout = _config.editor_config.password_editor
      ? loadLayout(R.xml.latn_qwerty_us) : refresh_special_layout();
    _keyeventhandler.started(_config);
    apply_current_layout();
  }

  @Override
  public void onStartInputView(EditorInfo info, boolean restarting)
  {
    if (!physical_mode_selected())
      prepare_input();
    setInputView(_keyboard_container_view);
    if (_floating_candidates != null)
      track_floating_caret(true);
    Logs.debug_startup_input_view(info, _config);
    if (!physical_mode_selected())
      startClipboardPolling();
  }

  @Override
  public void onFinishInput()
  {
    _physical_shift_down = null;
    if (physical_input_active())
      _keyeventhandler.cancel_composition();
    if (_floating_candidates != null)
      _floating_candidates.set_state(false, false);
    super.onFinishInput();
  }

  private void startClipboardPolling()
  {
    stopClipboardPolling();
    if (_config.editor_config.password_editor)
      return;
    try
    {
      if (!new ClipboardSyncClient.Account(this).configured())
        return;
    }
    catch (Exception _e) { return; }

    final int generation = _clipboardPollGeneration;
    _clipboardPollTask = new Runnable()
    {
      @Override public void run()
      {
        if (generation != _clipboardPollGeneration
            || _clipboardPollTask == null)
          return;
        if (_clipboardPollInFlight)
        {
          _handler.postDelayed(this, CLIPBOARD_POLL_INTERVAL_MS);
          return;
        }
        _clipboardPollInFlight = true;
        ClipboardSyncClient.pull(Keyboard2.this,
            new ClipboardSyncClient.Callback()
            {
              @Override public void complete(ClipboardSyncClient.Result result)
              {
                _clipboardPollInFlight = false;
                if (generation != _clipboardPollGeneration)
                  return;
                if (!result.duplicate && result.file != null)
                  ClipboardHistoryService.set_current_clip_uri(
                      Keyboard2.this, result.file.uri, result.file.name);
                else if (!result.duplicate && result.text != null)
                  ClipboardHistoryService.set_current_clip_text(
                      Keyboard2.this, result.text);
                if (_clipboardPollTask != null)
                  _handler.postDelayed(_clipboardPollTask,
                      CLIPBOARD_POLL_INTERVAL_MS);
              }
            });
      }
    };
    _handler.post(_clipboardPollTask);
  }

  private void stopClipboardPolling()
  {
    _clipboardPollGeneration++;
    if (_clipboardPollTask != null)
      _handler.removeCallbacks(_clipboardPollTask);
    _clipboardPollTask = null;
  }

  @Override
  public void setExtractViewShown(boolean shown){
      super.setExtractViewShown(false);
  }

  @Override
  public void setInputView(View v)
  {
    if (_floating_candidates != null)
    {
      _floating_candidates.detach_card();
      _floating_candidates = null;
    }
    ViewParent parent = v.getParent();
    if (parent != null && parent instanceof ViewGroup)
      ((ViewGroup)parent).removeView(v);
    if (v == _keyboard_container_view && physical_input_active())
    {
      _floating_candidates = new FloatingCandidates(v, this::toggle_physical_language);
      update_floating_candidates();
      v = _floating_candidates;
    }
    super.setInputView(v);
    updateSoftInputWindowLayoutParams();
    v.requestApplyInsets();
  }

  private void update_floating_candidates()
  {
    if (_floating_candidates != null)
      _floating_candidates.set_state(physical_candidates_enabled(), _keyeventhandler.has_composition());
  }

  @Override
  public void onComputeInsets(Insets outInsets)
  {
    if (_floating_candidates == null || !isInputViewShown())
    {
      super.onComputeInsets(outInsets);
      return;
    }
    // Reserve zero space in the app and let touches outside the card through.
    int bottom = getWindow().getWindow().getDecorView().getHeight();
    outInsets.contentTopInsets = bottom;
    outInsets.visibleTopInsets = bottom;
    outInsets.touchableInsets = Insets.TOUCHABLE_INSETS_REGION;
    _floating_candidates.set_touchable_region(outInsets.touchableRegion);
  }

  private void track_floating_caret(boolean enabled)
  {
    InputConnection connection = getCurrentInputConnection();
    if (connection == null)
      return;
    int mode = enabled
      ? InputConnection.CURSOR_UPDATE_IMMEDIATE | InputConnection.CURSOR_UPDATE_MONITOR : 0;
    if (!enabled || VERSION.SDK_INT < 33 || !connection.requestCursorUpdates(
          mode, InputConnection.CURSOR_UPDATE_FILTER_INSERTION_MARKER))
      connection.requestCursorUpdates(mode);
  }

  @Override
  public void onUpdateCursorAnchorInfo(CursorAnchorInfo info)
  {
    if (_floating_candidates != null && isInputViewShown())
      _floating_candidates.set_cursor_anchor(info);
  }

  @Override
  public void updateFullscreenMode() {
    super.updateFullscreenMode();
    updateSoftInputWindowLayoutParams();
  }

  private void updateSoftInputWindowLayoutParams() {
    final Window window = getWindow().getWindow();
    // On API >= 35, Keyboard2View behaves as edge-to-edge
    // APIs 30 to 34 have visual artifact when edge-to-edge is enabled
    if (VERSION.SDK_INT >= 35)
    {
      WindowManager.LayoutParams wattrs = window.getAttributes();
      wattrs.layoutInDisplayCutoutMode =
        WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
      // Allow to draw behind system bars
      wattrs.setFitInsetsTypes(0);
      window.setDecorFitsSystemWindows(false);
    }
    updateLayoutHeightOf(window, ViewGroup.LayoutParams.MATCH_PARENT);
    final View inputArea = window.findViewById(android.R.id.inputArea);

    updateLayoutHeightOf(
            (View) inputArea.getParent(),
            isFullscreenMode()
                    ? ViewGroup.LayoutParams.MATCH_PARENT
                    : ViewGroup.LayoutParams.WRAP_CONTENT);
    updateLayoutGravityOf((View) inputArea.getParent(), Gravity.BOTTOM);

  }

  private static void updateLayoutHeightOf(final Window window, final int layoutHeight) {
    final WindowManager.LayoutParams params = window.getAttributes();
    if (params != null && params.height != layoutHeight) {
      params.height = layoutHeight;
      window.setAttributes(params);
    }
  }

  private static void updateLayoutHeightOf(final View view, final int layoutHeight) {
    final ViewGroup.LayoutParams params = view.getLayoutParams();
    if (params != null && params.height != layoutHeight) {
      params.height = layoutHeight;
      view.setLayoutParams(params);
    }
  }

  private static void updateLayoutGravityOf(final View view, final int layoutGravity) {
    final ViewGroup.LayoutParams lp = view.getLayoutParams();
    if (lp instanceof LinearLayout.LayoutParams) {
      final LinearLayout.LayoutParams params = (LinearLayout.LayoutParams) lp;
      if (params.gravity != layoutGravity) {
        params.gravity = layoutGravity;
        view.setLayoutParams(params);
      }
    } else if (lp instanceof FrameLayout.LayoutParams) {
      final FrameLayout.LayoutParams params = (FrameLayout.LayoutParams) lp;
      if (params.gravity != layoutGravity) {
        params.gravity = layoutGravity;
        view.setLayoutParams(params);
      }
    }
  }

  @Override
  public void onCurrentInputMethodSubtypeChanged(InputMethodSubtype subtype)
  {
    refreshSubtypeImm();
    refresh_current_dictionary();
    apply_current_layout();
  }

  @Override
  public void onUpdateSelection(int oldSelStart, int oldSelEnd, int newSelStart, int newSelEnd, int candidatesStart, int candidatesEnd)
  {
    super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd);
    _keyeventhandler.selection_updated(oldSelStart, newSelStart, newSelEnd);
    if ((oldSelStart == oldSelEnd) != (newSelStart == newSelEnd))
      _keyboard_layout_view.set_selection_state(newSelStart != newSelEnd);
  }

  @Override
  public void onFinishInputView(boolean finishingInput)
  {
    stopClipboardPolling();
    if (_floating_candidates != null)
      track_floating_caret(false);
    if (physical_input_active())
      _keyeventhandler.cancel_composition();
    super.onFinishInputView(finishingInput);
    _keyboard_layout_view.reset();
  }

  @Override
  public void onSharedPreferenceChanged(SharedPreferences _prefs, String _key)
  {
    /* The rubytext toggle redraws the candidates itself. Reloading everything
       for its preference would clear the row and refill it, which blanks it
       for a frame at the instant of the tap. */
    if (Config.is_annotations_pref(_key))
      return;
    refresh_config();
    apply_current_layout();
    if ("physical_keyboard_behavior".equals(_key))
    {
      if (isInputViewShown())
      {
        if (_floating_candidates != null)
          track_floating_caret(false);
        setInputView(_keyboard_container_view);
        if (_floating_candidates != null)
          track_floating_caret(true);
      }
      updateInputViewShown();
    }
  }

  @Override
  public boolean onEvaluateFullscreenMode()
  {
    /* Entirely disable fullscreen mode. */
    return false;
  }

  @Override
  public boolean onEvaluateInputViewShown()
  {
    if (physical_mode_selected())
      return physical_candidates_enabled();
    // Since Android 16, this method returns [false] for unknown reasons.
    if (super.onEvaluateInputViewShown())
      return true;
    if (getResources().getConfiguration().hardKeyboardHidden
        == Configuration.HARDKEYBOARDHIDDEN_NO
        && (_config == null || "hide".equals(_config.physical_keyboard_behavior)))
    {
      Logs.debug("Physical keyboard is present");
      return false;
    }
    return true;
  }

  @Override
  public boolean onShowInputRequested(int flags, boolean configChange)
  {
    if (physical_mode_selected())
      return physical_candidates_enabled();
    return super.onShowInputRequested(flags, configChange);
  }

  private boolean physical_mode_selected()
  {
    Configuration config = getResources().getConfiguration();
    return _config != null && "compose".equals(_config.physical_keyboard_behavior)
      && config.keyboard == Configuration.KEYBOARD_QWERTY
      && config.hardKeyboardHidden == Configuration.HARDKEYBOARDHIDDEN_NO;
  }

  private boolean physical_input_active()
  {
    EditorInfo editor = getCurrentInputEditorInfo();
    return physical_mode_selected() && getCurrentInputStarted() && editor != null
      && (editor.inputType & InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_TEXT
      && !_config.editor_config.password_editor;
  }

  private boolean physical_candidates_enabled()
  {
    return _physical_chinese && physical_input_active();
  }

  private void toggle_physical_language()
  {
    if (!physical_input_active())
      return;
    _physical_chinese = !_physical_chinese;
    apply_current_layout();
    if (_physical_chinese)
      requestShowSelf(0);
    else
      requestHideSelf(0);
  }

  private static long physical_key_id(KeyEvent event)
  {
    return ((long)event.getDeviceId() << 32) | (event.getKeyCode() & 0xffffffffL);
  }

  @Override
  public boolean onKeyDown(int keyCode, KeyEvent event)
  {
    long id = physical_key_id(event);
    if (event.getRepeatCount() > 0)
    {
      if (_physical_shift_down != null && _physical_shift_down != id)
        _physical_shift_down = null;
      if (_physical_keys_down.contains(id))
      {
        if (physical_input_active() && _physical_chinese)
          _keyeventhandler.physical_key_down(event);
        return true;
      }
      return super.onKeyDown(keyCode, event);
    }
    _physical_shift_down = null;
    _physical_keys_down.remove(id);
    InputDevice device = event.getDevice();
    if (physical_input_active() && device != null && !device.isVirtual()
        && device.getKeyboardType() == InputDevice.KEYBOARD_TYPE_ALPHABETIC
        && event.isFromSource(InputDevice.SOURCE_KEYBOARD))
    {
      if (keyCode == KeyEvent.KEYCODE_SHIFT_LEFT || keyCode == KeyEvent.KEYCODE_SHIFT_RIGHT)
      {
        int modifier = keyCode == KeyEvent.KEYCODE_SHIFT_LEFT
          ? KeyEvent.META_SHIFT_LEFT_ON : KeyEvent.META_SHIFT_RIGHT_ON;
        if (event.hasNoModifiers() || event.hasModifiers(modifier))
          _physical_shift_down = id;
        // Let the editor receive both modifier events so Shift chords still work.
        return super.onKeyDown(keyCode, event);
      }
      if (!_physical_chinese || !_keyeventhandler.physical_key_down(event))
        return super.onKeyDown(keyCode, event);
      if (_keyeventhandler.has_composition() && !isInputViewShown())
        requestShowSelf(0);
      _physical_keys_down.add(id);
      return true;
    }
    return super.onKeyDown(keyCode, event);
  }

  @Override
  public boolean onKeyUp(int keyCode, KeyEvent event)
  {
    long id = physical_key_id(event);
    boolean switch_language = _physical_shift_down != null
      && _physical_shift_down == id && !event.isCanceled();
    _physical_shift_down = null;
    if (switch_language)
      toggle_physical_language();
    return _physical_keys_down.remove(id) || super.onKeyUp(keyCode, event);
  }

  public void launch_dictionaries_activity()
  {
    start_activity(DictionariesActivity.class);
  }

  /** Called from [onClick] attributes. */
  public void launch_dictionaries_activity(View v)
  {
    launch_dictionaries_activity();
  }

  void start_activity(Class cls)
  {
    Intent intent = new Intent(this, cls);
    intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
    startActivity(intent);
  }

  /** Not static */
  public class Receiver implements KeyEventHandler.IReceiver,
         KeyValue.Stateful.Symbol_provider, DictionarySwitcher.Callback
  {
    @Override
    public void page_candidates(int dir)
    {
      if (_candidates_view != null)
        _candidates_view.page_candidates(dir);
    }

    @Override
    public Composer.Candidate candidate_at_digit(int digit)
    {
      return (_candidates_view == null)
          ? null : _candidates_view.candidate_at_digit(digit);
    }

    public void handle_event_key(KeyValue.Event ev)
    {
      switch (ev)
      {
        case CONFIG:
          start_activity(SettingsActivity.class);
          break;

        case SWITCH_TEXT:
          setSpecialLayout(null);
          break;

        case SWITCH_NUMERIC:
          setSpecialLayout(loadNumericLayout());
          break;

        case SWITCH_EMOJI:
          if (_emojiPane == null)
            _emojiPane = (ViewGroup)inflate_view(R.layout.emoji_pane);
          else
            ((EmojiGridView)_emojiPane.findViewById(R.id.emoji_grid)).closeVariants();
          setInputView(_emojiPane);
          break;

        case SWITCH_CLIPBOARD:
          if (_clipboard_pane == null)
            _clipboard_pane = (ViewGroup)inflate_view(R.layout.clipboard_pane);
          setInputView(_clipboard_pane);
          break;

        case SWITCH_BACK_EMOJI:
        case SWITCH_BACK_CLIPBOARD:
          setInputView(_keyboard_container_view);
          break;

        case CHANGE_METHOD_PICKER:
          get_imm().showInputMethodPicker();
          break;

        case CHANGE_METHOD_PREV:
          if (VERSION.SDK_INT < 28)
            get_imm().switchToLastInputMethod(getConnectionToken());
          else
            switchToPreviousInputMethod();
          break;

        case CHANGE_METHOD_NEXT:
          if (VERSION.SDK_INT < 28)
            get_imm().switchToNextInputMethod(getConnectionToken(), false);
          else
            switchToNextInputMethod(false);
          break;

        case ACTION:
          InputConnection conn = getCurrentInputConnection();
          if (conn != null)
            conn.performEditorAction(_config.editor_config.actionId);
          break;

        case SWITCH_FORWARD:
          incrTextLayout(1);
          break;

        case SWITCH_BACKWARD:
          incrTextLayout(-1);
          break;

        case SWITCH_GREEKMATH:
          setSpecialLayout(loadNumpad(R.xml.greekmath));
          break;

        case CAPS_LOCK:
          set_shift_state(true, true);
          break;

        case SWITCH_VOICE_TYPING:
          if (!VoiceImeSwitcher.switch_to_voice_ime(Keyboard2.this, get_imm(),
                Config.globalPrefs()))
            _config.shouldOfferVoiceTyping = false;
          break;

        case SWITCH_VOICE_TYPING_CHOOSER:
          VoiceImeSwitcher.choose_voice_ime(Keyboard2.this, get_imm(),
              Config.globalPrefs());
          break;

        case HIDE_SELF:
          Keyboard2.this.requestHideSelf(0);
          break;

        case CHANGE_DICTIONARY:
          new DictionarySwitcher(Keyboard2.this, _dictionaries, this).choose();
          break;
      }
    }

    public void set_shift_state(boolean state, boolean lock)
    {
      _keyboard_layout_view.set_shift_state(state, lock);
    }

    public void set_compose_pending(boolean pending)
    {
      _keyboard_layout_view.set_compose_pending(pending);
    }

    public void selection_state_changed(boolean selection_is_ongoing)
    {
      _keyboard_layout_view.set_selection_state(selection_is_ongoing);
    }

    public InputConnection getCurrentInputConnection()
    {
      return Keyboard2.this.getCurrentInputConnection();
    }

    public boolean is_password_editor()
    {
      return _config != null && _config.editor_config.password_editor;
    }

    public Handler getHandler()
    {
      return _handler;
    }

    public void set_suggestions(Suggestions suggestions)
    {
      suggestions.composing = _keyeventhandler.composing_text();
      suggestions.composing_cursor = _keyeventhandler.composing_cursor();
      _candidates_view.set_candidates(suggestions, !_keyeventhandler.is_calculating());
      update_floating_candidates();
    }

    public String provide_stateful_key_symbol(KeyValue.Stateful q)
    {
      switch (q)
      {
        case Complete_first: return candidate_text(0);
        case Complete_second: return candidate_text(1);
        case Complete_third: return candidate_text(2);
        case Complete_emoji: return _suggestions.emoji_suggestion;
      }
      return "";
    }

    /** The text of the [i]th candidate, or [null] when there are less. */
    String candidate_text(int i)
    {
      return (i < _suggestions.candidates.size()) ?
        _suggestions.candidates.get(i).text : null;
    }

    public void on_change_dictionary(String dict_name)
    {
      select_dictionary(dict_name);
    }

    public void launch_dictionaries_activity()
    {
      Keyboard2.this.launch_dictionaries_activity();
    }
  }

  private IBinder getConnectionToken()
  {
    return getWindow().getWindow().getAttributes().token;
  }

  private View inflate_view(int layout)
  {
    return View.inflate(new ContextThemeWrapper(this, _config.theme), layout, null);
  }
}
