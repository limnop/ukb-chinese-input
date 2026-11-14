package smoke;
import android.app.Instrumentation;
import android.os.Bundle;
import juloo.keyboard2.staged.*;
import java.util.*;
public class Smoke extends Instrumentation {
 public void onCreate(Bundle b) { super.onCreate(b); start(); }
 private String readFile(java.io.File file) throws java.io.IOException {
  if(!file.exists())return "";
  try(java.io.FileInputStream in=new java.io.FileInputStream(file);java.io.ByteArrayOutputStream out=new java.io.ByteArrayOutputStream()) {
   byte[] b=new byte[4096];int n;while((n=in.read(b))!=-1)out.write(b,0,n);return out.toString("UTF-8");
  }
 }
 public void onStart() {
  Bundle result=new Bundle(); StringBuilder log=new StringBuilder();
  try {
   Pinyin.engine=null;
   Pinyin empty=new Pinyin(); empty.type("nihao");
   if(!empty.candidates().isEmpty()) throw new AssertionError("Legacy candidates still active");
   java.io.File history=new java.io.File(getTargetContext().getFilesDir(), "wowtype/history.tsv");
   String beforeHistory=readFile(history);
   long started=System.nanoTime();
   WowtypePinyinEngine.preload(getTargetContext());
   for(int i=0; i<600 && Pinyin.engine==null; i++) Thread.sleep(100);
   if(Pinyin.engine==null) throw new AssertionError("Native initialization failed");
   if(!beforeHistory.equals(readFile(history))) throw new AssertionError("Data update changed learning history");
   log.append("history_preserved=true\n");
   log.append("initialization_ms="+(System.nanoTime()-started)/1000000+"\n");
   log.append("source="+Pinyin.engine.getClass().getName()+"\n");
   android.content.SharedPreferences prefs = juloo.keyboard2.DirectBootAwarePreferences.get_shared_preferences(getTargetContext());
   juloo.keyboard2.Config.migrate(prefs);
   prefs.edit().putString("layouts", "[\"latn_pinyin_zh\"]").putInt("current_layout_portrait", 0).commit();
   String[] expected={"你好","中国","西安","我爱你","你好"};
   int caseIndex=0;
   String[] codes={"nihao","zhongguo","xi'an","woaini","nihap"};
   for(String code:codes){
    Pinyin p=new Pinyin(); p.set_context("今天"); p.type(code);
    long t=System.nanoTime(); List<Composer.Candidate> cs=p.candidates();
    if(cs.isEmpty()) throw new AssertionError("No candidates: "+code);
    if(!cs.get(0).text.equals(expected[caseIndex++])) throw new AssertionError("Wrong top candidate: "+code+" -> "+cs.get(0).text);
    log.append(code+" "+((System.nanoTime()-t)/1000000)+"ms: ");
    for(int i=0;i<Math.min(5,cs.size());i++) log.append(cs.get(i).text+"/"+cs.get(i).consumed+" ");
    log.append("\n");
    Composer.Candidate whole=null; for(Composer.Candidate c:cs) if(c.consumed==0){whole=c;break;}
    if(whole==null || !p.accept(whole).isEmpty()) throw new AssertionError("Whole commit failed");
   }
   Pinyin p=new Pinyin();p.type("nihaozhongguo");
   boolean partial=false;
   for(Composer.Candidate c:p.candidates()) if(c.consumed>0){String rest=p.accept(c); if(rest.isEmpty()||p.candidates().isEmpty())throw new AssertionError("Partial commit failed");partial=true;log.append("partial="+c.text+" remaining="+rest+"\n");break;}
   if(!partial) throw new AssertionError("No partial candidate");
   p=new Pinyin();p.type("nihao");p.backspace();if(!p.text().equals("niha")||p.candidates().isEmpty())throw new AssertionError("Backspace failed");
   p=new Pinyin();p.set_context("😀今天");p.type("ji");
   long queryStart=System.nanoTime();
   List<Composer.Candidate> expanded=p.candidates();
   boolean supplementary=false;
   for(Composer.Candidate c:expanded) if(c.text.equals("𬶭")) supplementary=true;
   if(!supplementary) throw new AssertionError("Supplementary Han candidate lost in JNI");
   if(expanded.size()<=100) throw new AssertionError("Long-tail candidates still capped at 100");
   log.append("ji candidates="+expanded.size()+" supplementary=𬶭 query_ms="+(System.nanoTime()-queryStart)/1000000+"\n");
   String[] sentences={"jintiantianqihenhao","woxiangqubeijing","zhonghuarenmingongheguo"};
   for(String code:sentences){
    p=new Pinyin();p.type(code);long begin=System.nanoTime();List<Composer.Candidate> cs=p.candidates();
    if(cs.isEmpty())throw new AssertionError("Missing sentence: "+code);
    log.append(code+" top="+cs.get(0).text+" ms="+(System.nanoTime()-begin)/1000000+"\n");
   }
   String[][] special={{"umumu","林"},{"uriyue","明"},{"uhuohuohuo","焱"},
       {"v1+2*3","7"},{"v(1+2)*3","9"},{"v123.45","壹佰贰拾叁元肆角伍分"}};
   String specialHistory=readFile(history);
   for(String[] sample:special){
    p=new Pinyin();if(!p.type(sample[0]))throw new AssertionError("Special input rejected");
    List<Composer.Candidate> cs=p.candidates();
    Composer.Candidate selected=null;
    for(Composer.Candidate c:cs) if(c.text.equals(sample[1])){selected=c;break;}
    if(selected==null || (sample[0].startsWith("v") && selected!=cs.get(0)))throw new AssertionError("Special candidate mismatch "+sample[0]);
    if(!p.accept(selected).isEmpty())throw new AssertionError("Special commit retained input");
    log.append(sample[0]+" -> "+sample[1]+"\n");
   }
   if(!specialHistory.equals(readFile(history)))throw new AssertionError("Special mode polluted pinyin history");
   p=new Pinyin();p.type("v1/0");
   if(!p.candidates().get(0).text.equals("1/0"))throw new AssertionError("Invalid arithmetic should remain literal");
   result.putString("stream",log+"PASS\n");finish(-1,result);
  } catch(Throwable e){result.putString("stream",log+"FAIL "+e);finish(0,result);}
 }
}
