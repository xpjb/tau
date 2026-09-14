import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.Map;
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
    private static final String SELECTION = "androidx/compose/foundation/text/selection/Selection";
    private static final String MANAGER = "androidx/compose/foundation/text/selection/SelectionManager";
    private static final Map<String, Set<String>> ORIGINALS = Map.of(
        "androidx/compose/foundation/text/selection/MultiWidgetSelectionDelegateKt.class", Set.of(
            "826fa451b8165ebe2d9b328317b19edcce4bf129739801698f18d18235b6f651",
            "3fc47482f8a985e2e4ed04c2683fbe63be8671e6cf55a94b97a1dbe11a6f5fa6"),
        MANAGER + ".class", Set.of(
            "07fb9c879d73f786066f9a9c63b0517eecc1b61f98d9109781f22d8b947d55d1",
            "e96c08bfe3091357ea70596ddb695b7c2c8d1f349fa9a70d0160c4ba2bc1e5e6")
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
            if (input.getName().equals("foundation.aar") && hash.equals("17c4466ca753a6ce68dcf4333674e0934d15c8246b223653d18f5dbaef0f6c10")) {
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
        int nestedJars = 0;
        Set<String> patchedClasses = new HashSet<>();
        try (ZipInputStream input = new ZipInputStream(new ByteArrayInputStream(archive));
             ZipOutputStream output = new ZipOutputStream(bytes)) {
            ZipEntry entry;
            while ((entry = input.getNextEntry()) != null) {
                byte[] content = input.readAllBytes();
                if (entry.getName().equals("classes.jar")) {
                    content = patchArchive(content);
                    nestedJars++;
                } else if (ORIGINALS.containsKey(entry.getName())) {
                    String hash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(content));
                    if (!ORIGINALS.get(entry.getName()).contains(hash)) {
                        throw new GradleException("Review the Compose selection patch for unknown class " + hash);
                    }
                    ClassReader reader = new ClassReader(content);
                    ClassWriter writer = new ClassWriter(reader, 0);
                    boolean manager = entry.getName().equals(MANAGER + ".class");
                    int[] changes = {0, 0, 0};
                    reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                        @Override
                        public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
                            MethodVisitor method = super.visitMethod(access, name, descriptor, signature, exceptions);
                            if (manager) {
                                if (!name.equals("updateSelection-jyLRC_s$foundation") ||
                                    !descriptor.equals("(JJZLandroidx/compose/foundation/text/selection/SelectionAdjustment;)Z")) return method;
                                return new MethodVisitor(Opcodes.ASM9, method) {
                                    @Override
                                    public void visitVarInsn(int opcode, int variable) {
                                        super.visitVarInsn(opcode, variable);
                                        if (opcode != Opcodes.ASTORE || variable != 9) return;
                                        changes[2]++;
                                        Object[] locals = {MANAGER, Opcodes.LONG, Opcodes.LONG, Opcodes.INTEGER,
                                            "androidx/compose/foundation/text/selection/SelectionAdjustment",
                                            "androidx/compose/foundation/text/selection/SelectionLayout", Opcodes.INTEGER, SELECTION};
                                        Label crossed = new Label();
                                        Label compare = new Label();
                                        Label done = new Label();
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION, "getStart", "()L" + SELECTION + "$AnchorInfo;", false);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION + "$AnchorInfo", "getSelectableId", "()J", false);
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION, "getEnd", "()L" + SELECTION + "$AnchorInfo;", false);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION + "$AnchorInfo", "getSelectableId", "()J", false);
                                        super.visitInsn(Opcodes.LCMP);
                                        super.visitJumpInsn(Opcodes.IFNE, done);
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION, "getStart", "()L" + SELECTION + "$AnchorInfo;", false);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION + "$AnchorInfo", "getOffset", "()I", false);
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION, "getEnd", "()L" + SELECTION + "$AnchorInfo;", false);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION + "$AnchorInfo", "getOffset", "()I", false);
                                        super.visitJumpInsn(Opcodes.IF_ICMPGT, crossed);
                                        super.visitInsn(Opcodes.ICONST_0);
                                        super.visitJumpInsn(Opcodes.GOTO, compare);
                                        super.visitLabel(crossed);
                                        super.visitFrame(Opcodes.F_NEW, locals.length, locals, 0, null);
                                        super.visitInsn(Opcodes.ICONST_1);
                                        super.visitLabel(compare);
                                        super.visitFrame(Opcodes.F_NEW, locals.length, locals, 1, new Object[] {Opcodes.INTEGER});
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION, "getHandlesCrossed", "()Z", false);
                                        super.visitJumpInsn(Opcodes.IF_ICMPEQ, done);
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitInsn(Opcodes.ACONST_NULL);
                                        super.visitInsn(Opcodes.ACONST_NULL);
                                        super.visitVarInsn(Opcodes.ALOAD, 9);
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, SELECTION, "getHandlesCrossed", "()Z", false);
                                        super.visitInsn(Opcodes.ICONST_1);
                                        super.visitInsn(Opcodes.IXOR);
                                        super.visitInsn(Opcodes.ICONST_3);
                                        super.visitInsn(Opcodes.ACONST_NULL);
                                        super.visitMethodInsn(Opcodes.INVOKESTATIC, SELECTION, "copy$default",
                                            "(L" + SELECTION + ";L" + SELECTION + "$AnchorInfo;L" + SELECTION + "$AnchorInfo;ZILjava/lang/Object;)L" + SELECTION + ";", false);
                                        super.visitVarInsn(Opcodes.ASTORE, 9);
                                        super.visitLabel(done);
                                        super.visitFrame(Opcodes.F_NEW, locals.length, locals, 0, null);
                                    }
                                };
                            }
                            if (!name.equals("getOffsetForPosition-3MmeM6k") ||
                                !descriptor.equals("(JLandroidx/compose/ui/text/TextLayoutResult;)I")) return method;
                            return new MethodVisitor(Opcodes.ASM9, method) {
                                @Override
                                public void visitMethodInsn(int opcode, String owner, String name, String descriptor, boolean isInterface) {
                                    if (owner.equals("androidx/compose/ui/text/TextLayoutResult") && name.equals("getMultiParagraph") && changes[1] == 0) {
                                        super.visitMethodInsn(Opcodes.INVOKEVIRTUAL, owner, "getSize-YbymL2g", "()J", false);
                                        changes[1]++;
                                    } else if (owner.equals("androidx/compose/ui/text/MultiParagraph") && name.equals("getHeight") && changes[1] == 1) {
                                        super.visitInsn(Opcodes.L2I);
                                        super.visitInsn(Opcodes.I2F);
                                        changes[1]++;
                                    } else {
                                        super.visitMethodInsn(opcode, owner, name, descriptor, isInterface);
                                    }
                                }
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
                    }, ClassReader.EXPAND_FRAMES);
                    if (manager ? changes[2] != 1 : changes[0] != 2 || changes[1] != 2) {
                        throw new GradleException("Compose selection method layout changed");
                    }
                    content = writer.toByteArray();
                    if (!patchedClasses.add(entry.getName())) throw new GradleException("Duplicate Compose selection class");
                }
                ZipEntry copy = new ZipEntry(entry.getName());
                copy.setTime(entry.getTime());
                output.putNextEntry(copy);
                output.write(content);
                output.closeEntry();
            }
        }
        if (!(nestedJars == 1 && patchedClasses.isEmpty()) &&
            !(nestedJars == 0 && patchedClasses.equals(ORIGINALS.keySet()))) {
            throw new GradleException("Expected both Compose selection classes or one classes.jar");
        }
        return bytes.toByteArray();
    }
}
