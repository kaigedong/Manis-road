import java.io.File;
import brut.androlib.smali.SmaliBuilder;
import com.android.tools.smali.dexlib2.Opcodes;
import com.android.tools.smali.dexlib2.writer.builder.DexBuilder;
import com.android.tools.smali.dexlib2.writer.io.FileDataStore;

class Assemble {
    public static void main(String[] args) throws Exception {
        var builder = new DexBuilder(Opcodes.forApi(28));
        if (!new SmaliBuilder(28).buildFile(new File(args[0]), builder)) {
            throw new IllegalStateException("smali assembly failed");
        }
        builder.writeTo(new FileDataStore(new File(args[1])));
    }
}
