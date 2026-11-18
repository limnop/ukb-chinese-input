package juloo.keyboard2.prefs;

import android.app.AlertDialog;
import android.content.Context;
import android.os.Bundle;
import android.preference.ListPreference;
import android.util.AttributeSet;
import android.view.ContextThemeWrapper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ArrayAdapter;
import android.widget.TextView;
import juloo.keyboard2.Config;
import juloo.keyboard2.R;

public class ThemePreference extends ListPreference
{
  public ThemePreference(Context context, AttributeSet attrs)
  {
    super(context, attrs);
  }

  private boolean isGroup(int position)
  {
    return getEntryValues()[position].toString().startsWith("group:");
  }

  @Override
  protected void showDialog(Bundle state)
  {
    super.showDialog(state);
    if (state == null)
    {
      int first = findIndexOfValue(getValue());
      while (first > 0 && !isGroup(first))
        first--;
      ((AlertDialog) getDialog()).getListView().setSelectionFromTop(Math.max(0, first), 0);
    }
  }

  @Override
  protected void onPrepareDialogBuilder(AlertDialog.Builder builder)
  {
    Context context = builder.getContext();
    CharSequence[] values = getEntryValues();
    ArrayAdapter<CharSequence> rows = new ArrayAdapter<CharSequence>(context,
        android.R.layout.simple_list_item_single_choice, getEntries()) {
      @Override public boolean areAllItemsEnabled() { return false; }
      @Override public boolean isEnabled(int position) { return getItemViewType(position) == 0; }
      @Override public int getViewTypeCount() { return 2; }
      @Override public int getItemViewType(int position)
      {
        return isGroup(position) ? 1 : 0;
      }

      @Override public View getView(int position, View recycled, ViewGroup parent)
      {
        if (!isEnabled(position))
        {
          TextView header = (TextView) recycled;
          if (header == null)
            header = (TextView) LayoutInflater.from(context).inflate(R.layout.theme_group_header, parent, false);
          header.setText(getItem(position));
          return header;
        }
        TextView row = (TextView) super.getView(position, recycled, parent);
        Context preview = new ContextThemeWrapper(context,
            Config.getThemeId(context.getResources(), values[position].toString()));
        row.setCompoundDrawablesRelativeWithIntrinsicBounds(preview.getDrawable(R.drawable.theme_swatch), null, null, null);
        row.setCompoundDrawablePadding(context.getResources().getDimensionPixelSize(R.dimen.theme_swatch_gap));
        return row;
      }
    };
    builder.setPositiveButton(null, null);
    builder.setSingleChoiceItems(rows, findIndexOfValue(getValue()), (dialog, position) -> {
      String value = values[position].toString();
      if (rows.isEnabled(position) && callChangeListener(value))
        setValue(value);
      dialog.dismiss();
    });
  }
}
