package juloo.keyboard2;

import android.content.res.AssetManager;
import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

enum NativeEmoji
{
  QQ("qq", R.string.emoji_app_qq),
  WECHAT("wechat", R.string.emoji_app_wechat),
  DOUYIN("douyin", R.string.emoji_app_douyin),
  BILIBILI("bilibili", R.string.emoji_app_bilibili);

  final int label;
  private final String _directory;
  private List<Emoji> _entries;

  NativeEmoji(String directory, int label)
  {
    _directory = "native-emoji/" + directory + "/";
    this.label = label;
  }

  static NativeEmoji forPackage(String packageName)
  {
    if (packageName == null)
      return null;
    switch (packageName)
    {
      case "com.tencent.mobileqq": return QQ;
      case "com.tencent.mm": return WECHAT;
      case "com.ss.android.ugc.aweme":
      case "com.ss.android.ugc.aweme.lite": return DOUYIN;
      case "tv.danmaku.bili":
      case "com.bilibili.app.in":
      case "tv.danmaku.bilibilihd": return BILIBILI;
      default: return null;
    }
  }

  String inputText(String code)
  {
    // QQ stores the Android local face ID in a single UTF-16 code unit.
    return this == QQ ? "\u0014" + (char)Integer.parseInt(code) : code;
  }

  List<Emoji> entries(AssetManager assets)
  {
    if (_entries != null)
      return _entries;
    List<Emoji> entries = new ArrayList<>();
    try (BufferedReader reader = new BufferedReader(new InputStreamReader(
            assets.open(_directory + "index.tsv"), StandardCharsets.UTF_8)))
    {
      String line;
      while ((line = reader.readLine()) != null)
      {
        String[] fields = line.split("\\t", -1);
        entries.add(new Emoji(inputText(fields[0]), fields[1], _directory + fields[2]));
      }
    }
    catch (IOException e) { Logs.exn("Cannot load emoji catalogue " + _directory, e); }
    _entries = Collections.unmodifiableList(entries);
    return _entries;
  }
}
