package app.tau.rust;

import android.app.NativeActivity;
import android.content.*;
import android.graphics.Color;
import android.net.Uri;
import android.os.*;
import android.provider.OpenableColumns;
import android.provider.MediaStore;
import android.webkit.MimeTypeMap;
import org.json.JSONObject;
import android.text.*;
import android.view.*;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.view.inputmethod.*;
import java.nio.charset.StandardCharsets;
import java.io.*;

/** OS bridges only: IME, clipboard, document grants, insets, task Back.
 * Connection, chat state, transcript, layout and rendering live in Rust. */
public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("tau_frontend"); }
    private static native boolean nativeBack();
    private static native void nativeResult(int kind, String first, String second);
    private static native void nativeViewport(int left, int top, int right, int bottom);
    private static native void nativeEdit(long id, long revision, String text,
        int start, int end, int composingStart, int composingEnd);
    private InlineInput input;
    private long inputId, inputRevision, inputRequest;
    private int inputLimit;
    private android.widget.PopupMenu inputMenu;
    private boolean updatingInput, imeVisible;
    private final android.graphics.Rect lastViewport = new android.graphics.Rect();
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        View decor = getWindow().getDecorView();
        decor.getViewTreeObserver().addOnGlobalLayoutListener(this::reportViewport);
        decor.setOnApplyWindowInsetsListener((view,insets) -> { reportViewport(insets); return insets; });
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            decor.setWindowInsetsAnimationCallback(new WindowInsetsAnimation.Callback(WindowInsetsAnimation.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
                @Override public WindowInsets onProgress(WindowInsets insets,java.util.List<WindowInsetsAnimation> running) {
                    reportViewport(insets); return insets;
                }
            });
        }
        decor.post(this::reportViewport);
        getWindow().setStatusBarColor(Color.rgb(9,13,18));
        getWindow().setNavigationBarColor(Color.rgb(9,13,18));
        getWindow().setNavigationBarContrastEnforced(false);
    }
    private InputMethodManager keyboard() { return (InputMethodManager)getSystemService(INPUT_METHOD_SERVICE); }
    private void reportViewport() { reportViewport(getWindow().getDecorView().getRootWindowInsets()); }
    @SuppressWarnings("deprecation") private void reportViewport(WindowInsets insets) {
        View decor = getWindow().getDecorView();
        android.graphics.Rect visible = new android.graphics.Rect();
        if (Build.VERSION.SDK_INT >= 30 && insets != null) {
            // Window bounds include system bars/IME even when the native
            // surface has already resized. Convert once to surface coordinates.
            visible.set(getWindowManager().getCurrentWindowMetrics().getBounds());
            android.graphics.Insets safe = insets.getInsets(WindowInsets.Type.systemBars()
                | WindowInsets.Type.displayCutout() | WindowInsets.Type.ime());
            visible.inset(safe.left,safe.top,safe.right,safe.bottom);
            imeVisible = insets.isVisible(WindowInsets.Type.ime());
        } else {
            decor.getWindowVisibleDisplayFrame(visible);
            if (insets != null) imeVisible = insets.getSystemWindowInsetBottom() > insets.getStableInsetBottom();
        }
        int[] origin = new int[2]; decor.getLocationOnScreen(origin);
        visible.offset(-origin[0],-origin[1]);
        if (visible.intersect(0,0,decor.getWidth(),decor.getHeight()) && !lastViewport.equals(visible)) {
            lastViewport.set(visible);
            nativeViewport(visible.left,visible.top,visible.right,visible.bottom);
        }
    }
    public void selectionHaptic() { runOnUiThread(() -> getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)); }
    public void background() { runOnUiThread(() -> moveTaskToBack(true)); }
    /** The NativeActivity owns this window's drawing surface. This transparent
     * view supplies ONLY Android's normal Editable/InputConnection; Rust draws
     * and hit-tests the actual inline field. No editor Dialog or second screen. */
    public void syncInput(String json) { runOnUiThread(() -> {
        try {
            if (json.equals("null")) {
                inputId = 0;
                if (inputMenu != null) { inputMenu.dismiss(); inputMenu = null; }
                if (input != null) {
                    keyboard().hideSoftInputFromWindow(input.getWindowToken(),0);
                    input.clearFocus(); input.setVisibility(View.GONE);
                }
                return;
            }
            JSONObject state = new JSONObject(json);
            inputLimit = state.getInt("max_bytes");
            if (input == null) {
                input = new InlineInput();
                input.setAlpha(0); input.setBackground(null);
                input.setShowSoftInputOnFocus(false);
                input.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
                input.setFilters(new InputFilter[] { (source,start,end,dest,dstart,dend) -> {
                    String candidate = dest.subSequence(0,dstart).toString()
                        + source.subSequence(start,end) + dest.subSequence(dend,dest.length());
                    return candidate.getBytes(StandardCharsets.UTF_8).length <= inputLimit
                        ? null : dest.subSequence(dstart,dend);
                }});
                input.addTextChangedListener(new TextWatcher() {
                    public void beforeTextChanged(CharSequence s,int start,int count,int after) {}
                    public void onTextChanged(CharSequence s,int start,int before,int count) {}
                    public void afterTextChanged(Editable value) { reportEdit(); }
                });
                input.setOnEditorActionListener((view,action,event) -> {
                    if (action == EditorInfo.IME_ACTION_DONE) { hideKeyboard(); return true; }
                    return false; // Multiline Enter inserts a newline, never sends.
                });
                addContentView(input,new FrameLayout.LayoutParams(1,1));
            }
            long id = state.getLong("id"), revision = state.getLong("revision"), request = state.getLong("request");
            boolean changed = id != inputId || revision != inputRevision;
            boolean show = request != inputRequest;
            updatingInput = true;
            if (changed) {
                if (inputMenu != null) { inputMenu.dismiss(); inputMenu = null; }
                boolean singleLine = state.getBoolean("single_line"), secret = state.getBoolean("secret");
                int type = InputType.TYPE_CLASS_TEXT | (secret ? InputType.TYPE_TEXT_VARIATION_PASSWORD
                    : singleLine ? 0 : InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES);
                input.setInputType(type); input.setSingleLine(singleLine);
                input.setImeOptions((singleLine ? EditorInfo.IME_ACTION_DONE : EditorInfo.IME_ACTION_NONE)
                    | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | (secret ? EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING : 0));
                input.setText(state.getString("text"));
                input.setSelection(Math.max(0,Math.min(input.length(),state.getInt("start"))),
                    Math.max(0,Math.min(input.length(),state.getInt("end"))));
                inputId = id; inputRevision = revision;
            }
            inputRequest = request;
            org.json.JSONArray rect = state.getJSONArray("rect");
            View content = findViewById(android.R.id.content);
            int[] at = new int[2], origin = new int[2];
            content.getLocationInWindow(at); getWindow().getDecorView().getLocationInWindow(origin);
            FrameLayout.LayoutParams layout = new FrameLayout.LayoutParams(
                Math.max(1,(int)Math.ceil(rect.getDouble(2))),Math.max(1,(int)Math.ceil(rect.getDouble(3))));
            layout.leftMargin = (int)Math.round(rect.getDouble(0)) + origin[0] - at[0];
            layout.topMargin = (int)Math.round(rect.getDouble(1)) + origin[1] - at[1];
            input.setLayoutParams(layout);
            input.setTextSize(android.util.TypedValue.COMPLEX_UNIT_PX,(float)state.getDouble("size"));
            int padding = Math.round((float)state.getDouble("size") * .75f);
            input.setPadding(padding,padding,padding,padding);
            input.setVisibility(View.VISIBLE); input.requestFocus();
            updatingInput = false;
            if (changed) keyboard().restartInput(input);
            if (show) input.post(() -> {
                if (inputId == id && inputRequest == request && input.hasFocus())
                    keyboard().showSoftInput(input,InputMethodManager.SHOW_IMPLICIT);
            });
        } catch (Exception e) {
            updatingInput = false;
            nativeResult(3,"Cannot focus the text input: " + e.getMessage(),"");
        }
    }); }
    private void reportEdit() {
        if (updatingInput || inputId == 0 || input == null) return;
        Editable value = input.getText();
        nativeEdit(inputId,inputRevision,value.toString(),input.getSelectionStart(),input.getSelectionEnd(),
            BaseInputConnection.getComposingSpanStart(value),BaseInputConnection.getComposingSpanEnd(value));
    }
    private final class InlineInput extends EditText {
        InlineInput() { super(MainActivity.this); }
        @Override protected void onSelectionChanged(int start,int end) { super.onSelectionChanged(start,end); reportEdit(); }
        // Native hit-testing owns touch/caret geometry. Never pan the window to
        // a second text layout when the IME tries to reveal its cursor.
        @Override public boolean requestRectangleOnScreen(android.graphics.Rect rectangle,boolean immediate) { return false; }
        @Override public boolean onTouchEvent(MotionEvent event) { return false; }
        @Override public InputConnection onCreateInputConnection(EditorInfo info) {
            InputConnection connection = super.onCreateInputConnection(info);
            if (connection == null) return null;
            if (Build.VERSION.SDK_INT >= 34) return new ReplacementConnection(connection);
            if (Build.VERSION.SDK_INT >= 33) return new AttributedConnection(connection);
            return new InlineConnection(connection);
        }
    }
    private class InlineConnection extends InputConnectionWrapper {
        private final long id = inputId, revision = inputRevision;
        InlineConnection(InputConnection connection) { super(connection,false); }
        boolean active() { return inputId == id && inputRevision == revision && id != 0; }
        boolean report(boolean result) { reportEdit(); return result; }
        @Override public boolean beginBatchEdit() { return active() && super.beginBatchEdit(); }
        @Override public boolean endBatchEdit() { return active() && report(super.endBatchEdit()); }
        @Override public boolean setComposingText(CharSequence text,int position) { return active() && report(super.setComposingText(text,position)); }
        @Override public boolean setComposingRegion(int start,int end) { return active() && report(super.setComposingRegion(start,end)); }
        @Override public boolean finishComposingText() { return active() && report(super.finishComposingText()); }
        @Override public boolean commitText(CharSequence text,int position) { return active() && report(super.commitText(text,position)); }
        @Override public boolean setSelection(int start,int end) { return active() && report(super.setSelection(start,end)); }
        @Override public boolean deleteSurroundingText(int before,int after) { return active() && report(super.deleteSurroundingText(before,after)); }
        @Override public boolean deleteSurroundingTextInCodePoints(int before,int after) { return active() && report(super.deleteSurroundingTextInCodePoints(before,after)); }
        // Handle IME-generated keys synchronously in this revision, not as an
        // untagged native key that might arrive after Send switched editors.
        @Override public boolean sendKeyEvent(KeyEvent event) { return active() && report(input.dispatchKeyEvent(event)); }
        @Override public boolean performContextMenuAction(int action) { return active() && report(super.performContextMenuAction(action)); }
        @Override public boolean performEditorAction(int action) { return active() && report(super.performEditorAction(action)); }
        @Override public boolean commitCompletion(CompletionInfo completion) { return active() && report(super.commitCompletion(completion)); }
        @Override public boolean commitCorrection(CorrectionInfo correction) { return active() && report(super.commitCorrection(correction)); }
        @Override public void closeConnection() { if (active()) super.closeConnection(); }
    }
    // Keep newer parameter types out of the class loaded on Android 10-12.
    private class AttributedConnection extends InlineConnection {
        AttributedConnection(InputConnection connection) { super(connection); }
        @Override public boolean commitText(CharSequence text,int position,TextAttribute attributes) { return active() && report(super.commitText(text,position,attributes)); }
        @Override public boolean setComposingText(CharSequence text,int position,TextAttribute attributes) { return active() && report(super.setComposingText(text,position,attributes)); }
        @Override public boolean setComposingRegion(int start,int end,TextAttribute attributes) { return active() && report(super.setComposingRegion(start,end,attributes)); }
    }
    private final class ReplacementConnection extends AttributedConnection {
        ReplacementConnection(InputConnection connection) { super(connection); }
        @Override public boolean replaceText(int start,int end,CharSequence text,int position,TextAttribute attributes) { return active() && report(super.replaceText(start,end,text,position,attributes)); }
    }
    public void inputMenu() { runOnUiThread(() -> {
        if (inputId == 0 || input == null) return;
        if (inputMenu != null) inputMenu.dismiss();
        final long id = inputId, revision = inputRevision;
        inputMenu = new android.widget.PopupMenu(this,input);
        boolean selected = input.getSelectionStart() != input.getSelectionEnd();
        boolean secret = input.getTransformationMethod() instanceof android.text.method.PasswordTransformationMethod;
        inputMenu.getMenu().add(0,android.R.id.cut,0,android.R.string.cut).setEnabled(selected && !secret);
        inputMenu.getMenu().add(0,android.R.id.copy,1,android.R.string.copy).setEnabled(selected && !secret);
        inputMenu.getMenu().add(0,android.R.id.paste,2,android.R.string.paste);
        inputMenu.getMenu().add(0,android.R.id.selectAll,3,android.R.string.selectAll);
        inputMenu.setOnMenuItemClickListener(item -> {
            if (inputId != id || inputRevision != revision) return false;
            if (item.getItemId() == android.R.id.selectAll) input.setSelection(0,input.length());
            else input.onTextContextMenuItem(item.getItemId());
            reportEdit(); return true;
        });
        inputMenu.show();
    }); }
    public void hideKeyboard() { runOnUiThread(() -> {
        if (input != null) keyboard().hideSoftInputFromWindow(input.getWindowToken(),0);
    }); }
    public void back() { runOnUiThread(() -> {
        reportViewport(); reportEdit();
        if (imeVisible) hideKeyboard(); else nativeBack();
    }); }
    public void copy(String text) { runOnUiThread(() -> ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE)).setPrimaryClip(ClipData.newPlainText("Tau",text))); }
    public void paste(long token) { runOnUiThread(() -> {
        ClipData data = ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE)).getPrimaryClip();
        if (data != null && data.getItemCount() > 0) nativeResult(2,data.getItemAt(0).coerceToText(this).toString(),Long.toString(token));
    }); }
    public void openUrl(String url) { runOnUiThread(() -> { try { startActivity(new Intent(Intent.ACTION_VIEW,Uri.parse(url))); } catch (Exception e) { nativeResult(3,"No application can open this link",""); } }); }
    public void pickFile() { runOnUiThread(() -> {
        try { Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT); i.addCategory(Intent.CATEGORY_OPENABLE); i.setType("*/*"); startActivityForResult(i,41); }
        catch (Exception e) { nativeResult(3,"No document picker is available",""); }
    }); }
    /** Save directly to the user's Downloads/Tau, not to the app or install directory. */
    public void exportFile(String key,String source,String name) {
        new Thread(() -> {
            String safe=name.replace('\\','/'); safe=safe.substring(safe.lastIndexOf('/')+1)
                .replaceAll("[\\x00-\\x1f]", "_");
            if (safe.isEmpty() || safe.equals(".") || safe.equals("..")) safe="tau-attachment";
            if (safe.length()>160) safe=safe.substring(0,160);
            String extension=safe.lastIndexOf('.')>=0 ? safe.substring(safe.lastIndexOf('.')+1).toLowerCase(java.util.Locale.ROOT) : "";
            String mime=MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
            if (mime==null) mime="application/octet-stream";
            ContentResolver resolver=getContentResolver();
            Uri uri=null;
            try {
                ContentValues values=new ContentValues();
                values.put(MediaStore.Downloads.DISPLAY_NAME,safe);
                values.put(MediaStore.Downloads.MIME_TYPE,mime);
                values.put(MediaStore.Downloads.RELATIVE_PATH,Environment.DIRECTORY_DOWNLOADS + "/Tau/");
                values.put(MediaStore.Downloads.IS_PENDING,1);
                uri=resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI,values);
                if (uri==null) throw new IOException("Cannot create the download");
                try (InputStream in=new FileInputStream(source); OutputStream out=resolver.openOutputStream(uri)) {
                    if (out==null) throw new IOException("Cannot write the download");
                    transfer(in,out,50000000);
                }
                values.clear(); values.put(MediaStore.Downloads.IS_PENDING,0);
                if (resolver.update(uri,values,null,null)!=1) throw new IOException("Cannot publish the download");
                String actual=safe;
                try (android.database.Cursor c=resolver.query(uri,new String[] {MediaStore.Downloads.DISPLAY_NAME},null,null,null)) {
                    if (c!=null && c.moveToFirst()) actual=c.getString(0);
                }
                JSONObject record=new JSONObject().put("location","Downloads/Tau/"+actual)
                    .put("reference",uri.toString()).put("mime_type",mime);
                nativeResult(4,key,record.toString());
            } catch (Exception e) {
                if (uri!=null) resolver.delete(uri,null,null);
                nativeResult(5,key,"Could not save to Downloads/Tau: "+e.getMessage());
            }
        },"tau-save-download").start();
    }
    public void openSaved(String reference,String mime,String identity,String lineage,String session,String entry) { runOnUiThread(() -> {
        try {
            Uri uri=Uri.parse(reference);
            try (android.content.res.AssetFileDescriptor descriptor=getContentResolver().openAssetFileDescriptor(uri,"r")) {
                if (descriptor==null) throw new FileNotFoundException("Download missing");
            }
            if (mime.equals("application/vnd.android.package-archive") && !getPackageManager().canRequestPackageInstalls()) {
                Intent settings=new Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:"+getPackageName()));
                startActivity(settings);return;
            }
            Intent intent=new Intent(Intent.ACTION_VIEW);
            intent.setDataAndType(uri,mime);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(intent);
        } catch (FileNotFoundException | SecurityException e) {
            try { nativeResult(6,identity,new org.json.JSONArray().put(lineage).put(session).put(entry).toString()); }
            catch (Exception ignored) { nativeResult(3,"The download is missing",""); }
        } catch (Exception e) { nativeResult(3,"Cannot open download: "+e.getMessage(),""); }
    }); }
    @Override protected void onActivityResult(int code,int result,Intent data) {
        super.onActivityResult(code,result,data);
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        final Uri uri = data.getData();
        new Thread(() -> {
            File pending = null;
            try {
                if (code == 41) {
                    String name = "attachment";
                    try (android.database.Cursor c = getContentResolver().query(uri,new String[] {OpenableColumns.DISPLAY_NAME},null,null,null)) {
                        if (c != null && c.moveToFirst()) name = c.getString(0);
                    }
                    pending = File.createTempFile("import-",".part",getCacheDir());
                    try (InputStream in = getContentResolver().openInputStream(uri); FileOutputStream out = new FileOutputStream(pending)) {
                        if (in == null) throw new IOException("Cannot read selected file");
                        transfer(in,out,50000000); out.getFD().sync();
                    }
                    nativeResult(1,pending.getAbsolutePath(),name == null ? "attachment" : name);
                }
            } catch (Exception e) { if (pending != null) pending.delete(); nativeResult(3,"File operation failed: " + e.getMessage(),""); }
        },"tau-documents").start();
    }
    private static void transfer(InputStream in,OutputStream out,long limit) throws IOException {
        byte[] buffer = new byte[65536]; long total = 0; int n;
        while ((n = in.read(buffer)) != -1) { total += n; if (total > limit) throw new IOException("File exceeds 50 MB"); out.write(buffer,0,n); }
    }
    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            if (event.getAction() == KeyEvent.ACTION_UP && !event.isCanceled()) back();
            return true;
        }
        return super.dispatchKeyEvent(event);
    }
    @Override @SuppressWarnings("deprecation") public void onBackPressed() { back(); }
}
