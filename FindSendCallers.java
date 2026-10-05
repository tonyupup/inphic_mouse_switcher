import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.address.Address;
import ghidra.util.task.ConsoleTaskMonitor;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.*;

public class FindSendCallers extends GhidraScript {
    @Override
    public void run() throws Exception {
        PrintWriter out = new PrintWriter(new FileWriter("/mnt/c/Users/z-l-8/yinfeike/mouse_switcher/ghidra_callers.txt"));
        FunctionManager fm = currentProgram.getFunctionManager();

        // FUN_0040cf60 is the generic checksum+hid_write sender we found last time.
        Address target = currentProgram.getAddressFactory().getAddress("0040cf60");
        Function sendFn = fm.getFunctionAt(target);
        out.println("send function: " + (sendFn != null ? sendFn.getName() : "NOT FOUND") + " @ " + target);

        DecompInterface decomp = new DecompInterface();
        decomp.openProgram(currentProgram);
        ConsoleTaskMonitor monitor = new ConsoleTaskMonitor();

        Set<Function> callers = new LinkedHashSet<Function>();
        for (Reference ref : currentProgram.getReferenceManager().getReferencesTo(target)) {
            Function fn = fm.getFunctionContaining(ref.getFromAddress());
            if (fn != null) callers.add(fn);
        }
        out.println("called from " + callers.size() + " distinct functions");
        out.println();

        for (Function fn : callers) {
            out.println("==================== caller: " + fn.getName() + " @ " + fn.getEntryPoint() + " ====================");
            DecompileResults res = decomp.decompileFunction(fn, 60, monitor);
            if (res.decompileCompleted()) {
                out.println(res.getDecompiledFunction().getC());
            } else {
                out.println("decompile failed: " + res.getErrorMessage());
            }
            out.println();
        }
        out.flush();
        out.close();
        println("DONE writing ghidra_callers.txt, " + callers.size() + " callers");
    }
}
