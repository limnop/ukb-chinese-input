package juloo.keyboard2;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.HashMap;
import java.util.Map;
import org.junit.Test;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

public class NativeEmojiTest
{
  @Test
  public void matchesOnlySupportedPackages()
  {
    assertEquals(NativeEmoji.QQ, NativeEmoji.forPackage("com.tencent.mobileqq"));
    assertEquals(NativeEmoji.WECHAT, NativeEmoji.forPackage("com.tencent.mm"));
    assertEquals(NativeEmoji.DOUYIN, NativeEmoji.forPackage("com.ss.android.ugc.aweme"));
    assertEquals(NativeEmoji.DOUYIN, NativeEmoji.forPackage("com.ss.android.ugc.aweme.lite"));
    assertEquals(NativeEmoji.BILIBILI, NativeEmoji.forPackage("tv.danmaku.bili"));
    assertEquals(NativeEmoji.BILIBILI, NativeEmoji.forPackage("tv.danmaku.bilibilihd"));
    assertEquals(NativeEmoji.BILIBILI, NativeEmoji.forPackage("com.bilibili.app.in"));
    assertNull(NativeEmoji.forPackage("com.tencent.mm.other"));
    assertNull(NativeEmoji.forPackage(null));
  }

  @Test
  public void preservesQqFaceIdsAndBracketCodes()
  {
    assertEquals("\u0014\u0000", NativeEmoji.QQ.inputText("0"));
    assertEquals("\u0014\u011b", NativeEmoji.QQ.inputText("283"));
    assertEquals("[旺柴]", NativeEmoji.WECHAT.inputText("[旺柴]"));
    assertEquals("[酷拽]", NativeEmoji.DOUYIN.inputText("[酷拽]"));
    assertEquals("[tv_doge]", NativeEmoji.BILIBILI.inputText("[tv_doge]"));
  }

  @Test
  public void qqPreviewsUseAndroidLocalCodes() throws IOException
  {
    Map<String, String> inputByImage = new HashMap<>();
    for (String row : Files.readAllLines(Paths.get("assets/native-emoji/qq/index.tsv"),
          StandardCharsets.UTF_8))
    {
      String[] fields = row.split("\\t");
      inputByImage.put(fields[2], NativeEmoji.QQ.inputText(fields[0]));
    }
    assertEquals("\u0014\u0021", inputByImage.get("0.webp")); // 惊讶
    assertEquals("\u0014\u0000", inputByImage.get("13.webp")); // 呲牙
    assertEquals("\u0014\u0017", inputByImage.get("14.webp")); // 微笑
    assertEquals("\u0014\u011b", inputByImage.get("283.webp")); // 狂笑
  }
}
