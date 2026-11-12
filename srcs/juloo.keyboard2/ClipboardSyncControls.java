package juloo.keyboard2;

import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.util.AttributeSet;
import android.util.TypedValue;
import android.view.View;
import android.widget.Button;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Manual text sync actions embedded in the existing clipboard pane. */
public final class ClipboardSyncControls extends LinearLayout
{
  TextView _status;
  LinearLayout _preview_panel;
  TextView _preview_text;
  ImageView _preview_image;
  Uri _preview_uri;
  String _preview_mime;

  public ClipboardSyncControls(Context context, AttributeSet attrs)
  {
    super(context, attrs);
    setOrientation(VERTICAL);
    setPadding(dp(8), dp(2), dp(8), dp(4));

    LinearLayout actions = new LinearLayout(context);
    actions.setOrientation(HORIZONTAL);
    addView(actions, new LayoutParams(LayoutParams.MATCH_PARENT,
        LayoutParams.WRAP_CONTENT));

    Button settings = add_button(actions, R.string.clipboard_sync_settings);
    Button pull = add_button(actions, R.string.clipboard_sync_download);
    Button push = add_button(actions, R.string.clipboard_sync_upload);
    settings.setOnClickListener(new View.OnClickListener()
        {
          @Override public void onClick(View v) { open_settings(); }
        });
    pull.setOnClickListener(new View.OnClickListener()
        {
          @Override public void onClick(View v) { download(); }
        });
    push.setOnClickListener(new View.OnClickListener()
        {
          @Override public void onClick(View v) { upload(); }
        });

    _status = new TextView(context);
    _status.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
    _status.setTextColor(0xff888888);
    addView(_status, new LayoutParams(LayoutParams.MATCH_PARENT,
        LayoutParams.WRAP_CONTENT));
    _preview_panel = new LinearLayout(context);
    _preview_panel.setOrientation(VERTICAL);
    _preview_panel.setVisibility(View.GONE);
    addView(_preview_panel, new LayoutParams(LayoutParams.MATCH_PARENT,
        LayoutParams.WRAP_CONTENT));
    _preview_image = new ImageView(context);
    _preview_image.setAdjustViewBounds(true);
    _preview_image.setMaxHeight(dp(220));
    _preview_image.setScaleType(ImageView.ScaleType.FIT_CENTER);
    _preview_panel.addView(_preview_image, new LayoutParams(
        LayoutParams.MATCH_PARENT, dp(180)));
    _preview_text = new TextView(context);
    _preview_text.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
    _preview_text.setMaxLines(8);
    _preview_panel.addView(_preview_text, new LayoutParams(
        LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT));
    Button open = new Button(context);
    open.setText(R.string.clipboard_sync_open_file);
    open.setOnClickListener(new View.OnClickListener()
        {
          @Override public void onClick(View v) { open_preview(); }
        });
    _preview_panel.addView(open, new LayoutParams(LayoutParams.WRAP_CONTENT,
        LayoutParams.WRAP_CONTENT));
    update_status();
  }

  Button add_button(LinearLayout row, int label)
  {
    Button button = new Button(getContext());
    button.setText(label);
    button.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
    button.setMinHeight(dp(40));
    button.setPadding(dp(4), 0, dp(4), 0);
    row.addView(button, new LayoutParams(0, LayoutParams.WRAP_CONTENT, 1));
    return button;
  }

  int dp(int value)
  {
    return (int)(value * getResources().getDisplayMetrics().density + 0.5f);
  }

  void update_status()
  {
    ClipboardSyncClient.Account account = new ClipboardSyncClient.Account(
        getContext().getApplicationContext());
    _status.setText(account.configured()
        ? getResources().getString(R.string.clipboard_sync_ready,
            account.url)
        : getResources().getString(R.string.clipboard_sync_not_configured));
  }

  void open_settings()
  {
    Intent intent = new Intent(getContext(), SettingsActivity.class);
    intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
    getContext().startActivity(intent);
  }

  void upload()
  {
    ClipData clip = ClipboardHistoryService.get_current_clip_data(getContext());
    if (clip == null || clip.getItemCount() == 0)
    {
      _status.setText(R.string.clipboard_sync_no_text);
      return;
    }
    _status.setText(R.string.clipboard_sync_uploading);
    ClipboardSyncClient.push(getContext(), clip,
        new ClipboardSyncClient.Callback()
        {
          @Override public void complete(ClipboardSyncClient.Result result)
          {
            _status.setText(result.message);
          }
        });
  }

  void download()
  {
    _status.setText(R.string.clipboard_sync_downloading);
    ClipboardSyncClient.pull(getContext(), new ClipboardSyncClient.Callback()
        {
          @Override public void complete(ClipboardSyncClient.Result result)
          {
            if (result.file != null)
            {
              ClipboardHistoryService.set_current_clip_uri(getContext(),
                  result.file.uri, result.file.name);
              _status.setText(result.message);
              show_file_preview(result);
            }
            else if (result.text != null)
            {
              ClipboardHistoryService.set_current_clip_text(getContext(),
                  result.text);
              _status.setText(result.message);
              hide_file_preview();
            }
            else
              _status.setText(result.message);
          }
        });
  }

  void show_file_preview(ClipboardSyncClient.Result result)
  {
    _preview_panel.setVisibility(View.VISIBLE);
    _preview_uri = result.file.uri;
    _preview_mime = result.file.mime;
    if (result.image_preview != null)
    {
      _preview_image.setImageBitmap(result.image_preview);
      _preview_image.setVisibility(View.VISIBLE);
    }
    else
    {
      _preview_image.setImageDrawable(null);
      _preview_image.setVisibility(View.GONE);
    }
    String info = getResources().getString(R.string.clipboard_sync_file_info,
        result.file.name, result.file.size);
    if (result.text_preview != null)
      info += "\n" + result.text_preview;
    _preview_text.setText(info);
  }

  void hide_file_preview()
  {
    _preview_panel.setVisibility(View.GONE);
    _preview_uri = null;
  }

  void open_preview()
  {
    if (_preview_uri == null)
      return;
    try
    {
      Intent intent = new Intent(Intent.ACTION_VIEW);
      intent.setDataAndType(_preview_uri,
          _preview_mime == null ? "*/*" : _preview_mime);
      intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
          | Intent.FLAG_ACTIVITY_NEW_TASK);
      getContext().startActivity(intent);
    }
    catch (Exception e)
    {
      _status.setText("无法打开该文件");
    }
  }
}
