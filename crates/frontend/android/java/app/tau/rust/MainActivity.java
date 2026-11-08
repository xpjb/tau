package app.tau.rust;

import io.github.xpjb.chad.ChadActivity;
import android.content.*;
import android.graphics.Color;
import android.net.Uri;
import android.os.*;
import android.provider.OpenableColumns;
import android.provider.MediaStore;
import android.view.*;
import android.webkit.MimeTypeMap;
import java.io.*;
import org.json.JSONObject;

/** Tau-specific clipboard, document, haptic, URL and task bridges. Chad owns
 * the NativeActivity text input, UTF conversion, generation fences and insets. */
public final class MainActivity extends ChadActivity {
    static { System.loadLibrary("tau_frontend"); }
    private static native boolean nativeBack();
    private static native void nativeResult(int kind, String first, String second);

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().setStatusBarColor(Color.rgb(9,13,18));
        getWindow().setNavigationBarColor(Color.rgb(9,13,18));
        getWindow().setNavigationBarContrastEnforced(false);
    }
    public void selectionHaptic() { runOnUiThread(() ->
        getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)); }
    public void background() { runOnUiThread(() -> moveTaskToBack(true)); }
    public void back() { runOnUiThread(() -> {
        if (!chadHideKeyboardIfVisible()) nativeBack();
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
