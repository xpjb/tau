package app.tau.rust;

import android.app.NativeActivity;
import android.app.Dialog;
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
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Button;
import android.view.inputmethod.InputMethodManager;
import java.io.*;

/** OS bridges only: IME, clipboard, document grants, insets, task Back.
 * Connection, chat state, transcript, layout and rendering live in Rust. */
public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("tau_frontend"); }
    private static native boolean nativeBack();
    private static native void nativeResult(int kind, String first, String second);
    private static native void nativeInsets(int left, int top, int right, int bottom);
    private Dialog editor;
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
    public void edit(String title, String value, boolean secret, boolean singleLine) { runOnUiThread(() -> {
        if (editor != null) return;
        Dialog dialog = new Dialog(this, android.R.style.Theme_Material_NoActionBar);
        editor = dialog;
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setBackgroundColor(Color.rgb(9,13,18));
        int padding = Math.round(16 * getResources().getDisplayMetrics().density);
        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        header.setPadding(padding,0,padding,0);
        TextView label = new TextView(this);
        label.setText(title); label.setTextSize(18); label.setTextColor(Color.WHITE);
        header.addView(label,new LinearLayout.LayoutParams(0,ViewGroup.LayoutParams.WRAP_CONTENT,1));
        Button done = new Button(this);
        done.setText("Done");
        header.addView(done);
        page.addView(header,new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,ViewGroup.LayoutParams.WRAP_CONTENT));
        EditText input = new EditText(this);
        input.setInputType(secret ? 129 : (singleLine ? android.text.InputType.TYPE_CLASS_TEXT : (android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE | android.text.InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)));
        input.setSingleLine(singleLine);
        input.setFilters(new InputFilter[] { new InputFilter.LengthFilter(262144) });
        input.setGravity(singleLine ? Gravity.CENTER_VERTICAL : Gravity.TOP);
        input.setTextColor(Color.WHITE); input.setTextSize(18);
        input.setPadding(padding,padding,padding,padding);
        input.setText(value); input.setSelection(input.length());
        input.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s,int st,int c,int a) {}
            public void onTextChanged(CharSequence s,int st,int before,int count) { nativeResult(0,s.toString(),""); }
            public void afterTextChanged(Editable e) {}
        });
        page.addView(input,new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,0,1));
        dialog.setContentView(page);
        dialog.setCanceledOnTouchOutside(false);
        Window window = dialog.getWindow();
        window.setBackgroundDrawable(new android.graphics.drawable.ColorDrawable(Color.rgb(9,13,18)));
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE | WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE);
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
            page.setOnApplyWindowInsetsListener((view,insets) -> {
                android.graphics.Insets safe = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout() | WindowInsets.Type.ime());
                view.setPadding(safe.left,safe.top,safe.right,safe.bottom);
                return insets;
            });
        } else page.setFitsSystemWindows(true);
        done.setOnClickListener(view -> {
            nativeResult(0,input.getText().toString(),"");
            dialog.dismiss();
        });
        dialog.setOnDismissListener(closed -> {
            ((InputMethodManager)getSystemService(INPUT_METHOD_SERVICE)).hideSoftInputFromWindow(input.getWindowToken(),0);
            if (editor == dialog) editor = null;
        });
        dialog.show();
        window.setLayout(ViewGroup.LayoutParams.MATCH_PARENT,ViewGroup.LayoutParams.MATCH_PARENT);
        page.requestApplyInsets();
        input.requestFocus();
        input.post(() -> ((InputMethodManager)getSystemService(INPUT_METHOD_SERVICE)).showSoftInput(input,InputMethodManager.SHOW_IMPLICIT));
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
        if (editor != null && editor.isShowing()) return super.dispatchKeyEvent(event);
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) { if (event.getAction() == KeyEvent.ACTION_UP && !event.isCanceled()) nativeBack(); return true; }
        return super.dispatchKeyEvent(event);
    }
    @Override @SuppressWarnings("deprecation") public void onBackPressed() {
        if (editor != null) editor.dismiss(); else nativeBack();
    }
}
