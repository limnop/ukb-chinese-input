package juloo.keyboard2.staged;
import static org.junit.Assert.*;
import java.io.StringReader;
import java.util.List;
import org.junit.Test;
public class ComponentLookupTest {
  @Test public void exactReadingsPrecedeFrequentCompletionsAndDeduplicate() throws Exception {
    ComponentLookup lookup=new ComponentLookup(new StringReader(
        "mumumu\t森\tmu'mu'mu\t1000\nmumu\t林\tmu'mu\t5\nmumu\t林\tmu'mu\t4\n"));
    List<Composer.Candidate> cs=lookup.candidates("mu'mu");
    assertEquals(2,cs.size());assertEquals("林",cs.get(0).text);
    assertEquals("mu'mu",cs.get(0).annotation);assertEquals("森",cs.get(1).text);
    assertEquals(0,cs.get(0).consumed);
    assertTrue(lookup.candidates("").isEmpty());assertTrue(lookup.candidates("unknown").isEmpty());
  }
}
