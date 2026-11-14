package smoke;
import android.app.Instrumentation;
import android.os.Bundle;
import juloo.keyboard2.staged.*;
import java.util.*;
public class Smoke extends Instrumentation {
 public void onCreate(Bundle b) { super.onCreate(b); start(); }
 public void onStart() {
  Bundle result=new Bundle(); StringBuilder log=new StringBuilder();
  try {
   Pinyin.engine=null;
   Pinyin empty=new Pinyin(); empty.type("nihao");
   if(!empty.candidates().isEmpty()) throw new AssertionError("Legacy candidates still active");
   WowtypePinyinEngine.preload(getTargetContext());
   for(int i=0; i<600 && Pinyin.engine==null; i++) Thread.sleep(100);
   if(Pinyin.engine==null) throw new AssertionError("Native initialization failed");
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
   result.putString("stream",log+"PASS\n");finish(-1,result);
  } catch(Throwable e){result.putString("stream",log+"FAIL "+e);finish(0,result);}
 }
}
