package juloo.keyboard2.suggestions;

import android.graphics.RectF;
import android.graphics.Region;
import android.graphics.drawable.Drawable;
import android.os.Build.VERSION;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inputmethod.CursorAnchorInfo;
import android.widget.FrameLayout;
import android.widget.TextView;
import juloo.keyboard2.R;

/** Positions the physical-keyboard candidates inside the IME window. */
public final class FloatingCandidates extends FrameLayout
{
  private final View _card;
  private final View _composing;
  private final TextView _badge;
  private View _popup;
  private final Drawable _card_background;
  private final Drawable _composing_background;
  private final int _gap;
  private final int[] _location = new int[2];
  private final RectF _caret = new RectF();

  public FloatingCandidates(View card, Runnable toggle_mode)
  {
    super(card.getContext());
    _card = card;
    _composing = card.findViewById(R.id.candidates_composing);
    _card_background = card.getBackground();
    _composing_background = _composing.getBackground();
    _gap = dp(8);
    LayoutParams params = new LayoutParams(LayoutParams.MATCH_PARENT,
        LayoutParams.WRAP_CONTENT, Gravity.BOTTOM);
    params.setMargins(_gap, 0, _gap, _gap);
    addView(card, params);
    card.setBackgroundResource(R.drawable.floating_candidates_background);
    _composing.setBackground(null);
    card.setClipToOutline(true);
    card.setElevation(dp(4));

    _badge = new TextView(getContext(), null, 0, R.style.candidates_text);
    _badge.setText("中");
    _badge.setTextSize(13);
    _badge.setGravity(Gravity.CENTER);
    _badge.setBackgroundResource(R.drawable.floating_candidates_background);
    _badge.setElevation(dp(4));
    _badge.setContentDescription(getResources().getString(R.string.physical_keyboard_mode_chinese));
    _badge.setOnClickListener(view -> toggle_mode.run());
    LayoutParams badge_params = new LayoutParams(dp(32), dp(32), Gravity.BOTTOM | Gravity.END);
    badge_params.setMargins(_gap, 0, _gap, _gap);
    addView(_badge, badge_params);
    set_opacity(_card_background.getAlpha());

    setOnApplyWindowInsetsListener((view, insets) -> {
      if (VERSION.SDK_INT >= 35)
      {
        android.graphics.Insets bars = insets.getInsets(
            WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
        setPadding(bars.left, bars.top, bars.right, bars.bottom);
      }
      return insets;
    });
    addOnLayoutChangeListener((view, l, t, r, b, oldL, oldT, oldR, oldB) ->
        post(this::place_card));
  }

  public void set_opacity(int opacity)
  {
    _card.getBackground().mutate().setAlpha(opacity);
    _badge.getBackground().mutate().setAlpha(opacity);
  }

  public void set_state(boolean chinese, boolean composing)
  {
    boolean show_card = chinese && composing && _popup == null;
    // Keep the row measured while idle so a digit can select immediately.
    _card.setVisibility(show_card ? View.VISIBLE : View.INVISIBLE);
    _badge.setVisibility(chinese && !composing && _popup == null ? View.VISIBLE : View.GONE);
    place_card();
  }

  public void set_popup(View popup)
  {
    if (_popup == popup)
      return;
    if (_popup != null)
      removeView(_popup);
    _popup = popup;
    if (popup != null)
    {
      LayoutParams params = new LayoutParams(LayoutParams.WRAP_CONTENT,
          LayoutParams.WRAP_CONTENT, Gravity.CENTER);
      popup.setBackgroundResource(R.drawable.floating_candidates_background);
      popup.getBackground().mutate().setAlpha(216);
      popup.setClipToOutline(true);
      popup.setElevation(dp(10));
      if (popup.getParent() instanceof ViewGroup)
        ((ViewGroup)popup.getParent()).removeView(popup);
      addView(popup, params);
    }
  }

  @Override
  protected void onMeasure(int widthSpec, int heightSpec)
  {
    if (_popup != null)
    {
      int width = MeasureSpec.getSize(widthSpec) - getPaddingLeft() - getPaddingRight();
      _popup.getLayoutParams().width = Math.max(0, Math.min(dp(480), width - 2 * dp(24)));
    }
    super.onMeasure(widthSpec, heightSpec);
  }

  public void set_cursor_anchor(CursorAnchorInfo info)
  {
    float x = info.getInsertionMarkerHorizontal();
    _caret.set(x, info.getInsertionMarkerTop(), x + 1, info.getInsertionMarkerBottom());
    info.getMatrix().mapRect(_caret);
    place_card();
  }

  /** Keep the usual bottom position unless the editor caret would be under
      the card. Compute from the resting position so layout cannot oscillate. */
  private void place_card()
  {
    int height = _card.getHeight();
    if (_card.getVisibility() != View.VISIBLE || height == 0 || !isAttachedToWindow())
      return;
    getLocationOnScreen(_location);
    int resting_bottom = _location[1] + getHeight() - getPaddingBottom() - _gap;
    int resting_top = resting_bottom - height;
    int lift = 0;
    if (_caret.top < _caret.bottom && _caret.bottom + _gap > resting_top
        && _caret.top - _gap < resting_bottom)
    {
      int above = (int)Math.floor(_caret.top) - _gap;
      View root = getRootView();
      root.getLocationOnScreen(_location);
      if (above - height >= _location[1] + _gap)
        lift = resting_bottom - above;
    }
    LayoutParams params = (LayoutParams)_card.getLayoutParams();
    if (params.bottomMargin != _gap + lift)
    {
      params.bottomMargin = _gap + lift;
      _card.setLayoutParams(params);
    }
  }

  public void set_touchable_region(Region region)
  {
    View visible = _popup != null ? _popup
        : (_card.getVisibility() == View.VISIBLE ? _card : _badge);
    if (visible.getVisibility() != View.VISIBLE)
    {
      region.setEmpty();
      return;
    }
    visible.getLocationInWindow(_location);
    region.set(_location[0], _location[1],
        _location[0] + visible.getWidth(), _location[1] + visible.getHeight());
  }

  public void detach_card()
  {
    set_popup(null);
    removeView(_card);
    _card_background.setAlpha(_card.getBackground().getAlpha());
    _card.setBackground(_card_background);
    _composing.setBackground(_composing_background);
    _card.setVisibility(View.VISIBLE);
    _card.setClipToOutline(false);
    _card.setElevation(0);
  }

  private int dp(int value)
  {
    return Math.round(value * getResources().getDisplayMetrics().density);
  }
}
