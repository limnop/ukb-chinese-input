package juloo.keyboard2;

import android.app.AlertDialog;
import android.content.Context;
import android.graphics.Typeface;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import java.util.ArrayList;
import java.util.List;

/** Lets the user paste whitespace- or punctuation-separated pieces one by one. */
final class ClipboardTextSegmentDialog
{
  static void show(Context context, View anchor, String text)
  {
    final List<String> segments = split(text);
    if (segments.size() < 2)
    {
      Toast.makeText(context, R.string.clipboard_split_no_segments,
          Toast.LENGTH_SHORT).show();
      return;
    }

    final LinearLayout items = new LinearLayout(context);
    items.setOrientation(LinearLayout.VERTICAL);
    int pad = dp(context, 12);
    items.setPadding(pad, dp(context, 4), pad, dp(context, 4));
    ScrollView scroll = new ScrollView(context);
    scroll.addView(items, new ViewGroup.LayoutParams(
          ViewGroup.LayoutParams.MATCH_PARENT,
          ViewGroup.LayoutParams.WRAP_CONTENT));
    final AlertDialog dialog = new AlertDialog.Builder(context)
      .setTitle(R.string.clipboard_split_title)
      .setView(scroll)
      .setNegativeButton(android.R.string.cancel, null)
      .create();

    for (final String segment : segments)
    {
      TextView item = new TextView(context);
      item.setText(segment);
      item.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
      item.setTypeface(null, Typeface.BOLD);
      item.setPadding(dp(context, 8), dp(context, 12), dp(context, 8),
          dp(context, 12));
      item.setBackgroundResource(android.R.drawable.list_selector_background);
      item.setOnClickListener(new View.OnClickListener()
          {
            @Override public void onClick(View v)
            {
              ClipboardHistoryService.paste(segment);
              items.removeView(v);
              if (items.getChildCount() == 0)
                dialog.dismiss();
            }
          });
      items.addView(item, new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT));
    }
    Utils.show_dialog_on_ime(dialog, anchor.getWindowToken());
  }

  static List<String> split(String text)
  {
    List<String> out = new ArrayList<String>();
    if (text == null)
      return out;
    String[] chunks = text.split("(?<=[，,。；;：:！？!?])|\\s+");
    for (String chunk : chunks)
    {
      String clean = chunk.trim();
      if (clean.length() > 0)
        out.add(clean);
    }
    return out;
  }

  static int dp(Context context, int value)
  {
    return (int)(value * context.getResources().getDisplayMetrics().density
        + 0.5f);
  }
}
