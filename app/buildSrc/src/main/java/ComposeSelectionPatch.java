import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.Set;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;
import java.util.zip.ZipOutputStream;
import org.gradle.api.GradleException;
import org.gradle.api.artifacts.transform.CacheableTransform;
import org.gradle.api.artifacts.transform.InputArtifact;
import org.gradle.api.artifacts.transform.TransformAction;
import org.gradle.api.artifacts.transform.TransformOutputs;
import org.gradle.api.artifacts.transform.TransformParameters;
import org.gradle.api.file.FileSystemLocation;
import org.gradle.api.provider.Provider;
import org.gradle.api.tasks.PathSensitive;
import org.gradle.api.tasks.PathSensitivity;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

@CacheableTransform
public abstract class ComposeSelectionPatch implements TransformAction<TransformParameters.None> {
    private static final String TARGET = "androidx/compose/foundation/text/selection/MultiWidgetSelectionDelegateKt.class";
    private static final Set<String> ORIGINALS = Set.of(
        "826fa451b8165ebe2d9b328317b19edcce4bf129739801698f18d18235b6f651",
        "3fc47482f8a985e2e4ed04c2683fbe63be8671e6cf55a94b97a1dbe11a6f5fa6"
    );

    @InputArtifact
    @PathSensitive(PathSensitivity.NAME_ONLY)
    public abstract Provider<FileSystemLocation> getInputArtifact();

    @Override
    public void transform(TransformOutputs outputs) {
        File input = getInputArtifact().get().getAsFile();
        if (!input.getName().equals("foundation-desktop-1.12.0.jar") && !input.getName().equals("foundation.aar")) {
            outputs.file(input);
            return;
        }
        try {
            byte[] original = Files.readAllBytes(input.toPath());
            String hash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(original));
            if (hash.equals("17c4466ca753a6ce68dcf4333674e0934d15c8246b223653d18f5dbaef0f6c10")) {
                outputs.file(input);
                return;
            }
            byte[] patched = patchArchive(original);
            Files.write(outputs.file(input.getName()).toPath(), patched);
        } catch (IOException | NoSuchAlgorithmException error) {
            throw new GradleException("Cannot patch Compose selection: " + input, error);
        }
    }

    private static byte[] patchArchive(byte[] archive) throws IOException, NoSuchAlgorithmException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        int patchedEntries = 0;
        try (ZipInputStream input = new ZipInputStream(new ByteArrayInputStream(archive));
             ZipOutputStream output = new ZipOutputStream(bytes)) {
            ZipEntry entry;
            while ((entry = input.getNextEntry()) != null) {
                byte[] content = input.readAllBytes();
                if (entry.getName().equals("classes.jar")) {
                    content = patchArchive(content);
                    patchedEntries++;
                } else if (entry.getName().equals(TARGET)) {
                    String hash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(content));
                    if (!ORIGINALS.contains(hash)) {
                        throw new GradleException("Review the Compose selection patch for unknown class " + hash);
                    }
                    ClassReader reader = new ClassReader(content);
                    ClassWriter writer = new ClassWriter(reader, 0);
                    int[] changes = {0};
                    reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                        @Override
                        public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
                            MethodVisitor method = super.visitMethod(access, name, descriptor, signature, exceptions);
                            if (!name.equals("getOffsetForPosition-3MmeM6k") ||
                                !descriptor.equals("(JLandroidx/compose/ui/text/TextLayoutResult;)I")) return method;
                            return new MethodVisitor(Opcodes.ASM9, method) {
                                @Override
                                public void visitJumpInsn(int opcode, Label label) {
                                    if (opcode == Opcodes.IFGT && changes[0] == 0) {
                                        opcode = Opcodes.IFGE;
                                        changes[0]++;
                                    } else if (opcode == Opcodes.IFLT && changes[0] == 1) {
                                        opcode = Opcodes.IFLE;
                                        changes[0]++;
                                    }
                                    super.visitJumpInsn(opcode, label);
                                }
                            };
                        }
                    }, 0);
                    if (changes[0] != 2) throw new GradleException("Compose selection comparison layout changed");
                    content = writer.toByteArray();
                    patchedEntries++;
                }
                ZipEntry copy = new ZipEntry(entry.getName());
                copy.setTime(entry.getTime());
                output.putNextEntry(copy);
                output.write(content);
                output.closeEntry();
            }
        }
        if (patchedEntries != 1) throw new GradleException("Expected one Compose selection class or classes.jar, found " + patchedEntries);
        return bytes.toByteArray();
    }
}
