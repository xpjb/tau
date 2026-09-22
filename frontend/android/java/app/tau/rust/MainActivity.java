package app.tau.rust;

import android.app.NativeActivity;
import android.app.AlertDialog;
import android.content.*;
import android.graphics.Color;
import android.net.Uri;
import android.os.*;
import android.provider.OpenableColumns;
import android.text.*;
import android.view.*;
import android.widget.EditText;
import java.io.*;

/** OS bridges only: IME, clipboard, document grants, insets, task Back.
 * Connection, chat state, transcript, layout and rendering live in Rust. */
public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("tau_frontend"); }
    private static native boolean nativeBack();
    private static native void nativeResult(int kind, String first, String second);
    private static native void nativeInsets(int left, int top, int right, int bottom);
    private String exportSource;
    private AlertDialog editor;
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        View decor = getWindow().getDecorView();
        decor.getViewTreeObserver().addOnGlobalLayoutListener(this::reportInsets);
        decor.setOnApplyWindowInsetsListener((view,insets) -> { reportInsets(); return insets; });
        decor.post(this::reportInsets);
        getWindow().setStatusBarColor(Color.rgb(9,13,18));
        getWindow().setNavigationBarColor(Color.rgb(9,13,18));
        getWindow().setNavigationBarContrastEnforced(false);
    }
    @SuppressWarnings("deprecation") private void reportInsets() {
        WindowInsets i = getWindow().getDecorView().getRootWindowInsets(); if (i == null) return;
        if (Build.VERSION.SDK_INT >= 30) {
            android.graphics.Insets s = i.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            nativeInsets(s.left,s.top,s.right,s.bottom);
        } else nativeInsets(i.getSystemWindowInsetLeft(),i.getSystemWindowInsetTop(),i.getSystemWindowInsetRight(),i.getSystemWindowInsetBottom());
    }
    public void background() { runOnUiThread(() -> moveTaskToBack(true)); }
    public void edit(String title, String value, boolean secret) { runOnUiThread(() -> {
        if (editor != null) editor.dismiss();
        EditText input = new EditText(this);
        input.setInputType(secret ? 129 : (android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE | android.text.InputType.TYPE_TEXT_FLAG_CAP_SENTENCES));
        input.setFilters(new InputFilter[] { new InputFilter.LengthFilter(262144) });
        input.setMinLines(secret ? 1 : 3); input.setMaxLines(10); input.setText(value); input.setSelection(input.length());
        input.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s,int st,int c,int a) {}
            public void onTextChanged(CharSequence s,int st,int before,int count) { nativeResult(0,s.toString(),""); }
            public void afterTextChanged(Editable e) {}
        });
        editor = new AlertDialog.Builder(this).setTitle(title).setView(input).setPositiveButton("Done",(dialog,which) -> {}).create();
        editor.setOnShowListener(dialog -> { input.requestFocus(); editor.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE); });
        editor.show();
    }); }
    public void copy(String text) { runOnUiThread(() -> ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE)).setPrimaryClip(ClipData.newPlainText("Tau",text))); }
    public void paste() { runOnUiThread(() -> {
        ClipData data = ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE)).getPrimaryClip();
        if (data != null && data.getItemCount() > 0) nativeResult(2,data.getItemAt(0).coerceToText(this).toString(),"");
    }); }
    public void openUrl(String url) { runOnUiThread(() -> { try { startActivity(new Intent(Intent.ACTION_VIEW,Uri.parse(url))); } catch (Exception e) { nativeResult(3,"No application can open this link",""); } }); }
    public void pickFile() { runOnUiThread(() -> {
        try { Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT); i.addCategory(Intent.CATEGORY_OPENABLE); i.setType("*/*"); startActivityForResult(i,41); }
        catch (Exception e) { nativeResult(3,"No document picker is available",""); }
    }); }
    public void exportFile(String source,String name) { runOnUiThread(() -> {
        exportSource = source;
        try { Intent i = new Intent(Intent.ACTION_CREATE_DOCUMENT); i.addCategory(Intent.CATEGORY_OPENABLE); i.setType("application/octet-stream"); i.putExtra(Intent.EXTRA_TITLE,new File(name).getName()); startActivityForResult(i,42); }
        catch (Exception e) { nativeResult(3,"No document exporter is available",""); }
    }); }
    @Override protected void onActivityResult(int code,int result,Intent data) {
        super.onActivityResult(code,result,data);
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        final Uri uri = data.getData(); final String source = exportSource;
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
                } else if (code == 42 && source != null) {
                    try (InputStream in = new FileInputStream(source); OutputStream out = getContentResolver().openOutputStream(uri,"wt")) {
                        if (out == null) throw new IOException("Cannot write selected destination"); transfer(in,out,50000000);
                    }
                }
            } catch (Exception e) { if (pending != null) pending.delete(); nativeResult(3,"File operation failed: " + e.getMessage(),""); }
        },"tau-documents").start();
    }
    private static void transfer(InputStream in,OutputStream out,long limit) throws IOException {
        byte[] buffer = new byte[65536]; long total = 0; int n;
        while ((n = in.read(buffer)) != -1) { total += n; if (total > limit) throw new IOException("File exceeds 50 MB"); out.write(buffer,0,n); }
    }
    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) { if (event.getAction() == KeyEvent.ACTION_UP && !event.isCanceled()) nativeBack(); return true; }
        return super.dispatchKeyEvent(event);
    }
    @Override @SuppressWarnings("deprecation") public void onBackPressed() { nativeBack(); }
}
