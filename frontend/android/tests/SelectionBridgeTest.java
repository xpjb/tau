package app.tau.rust;

import android.app.*;
import android.content.*;
import android.os.*;
import android.view.*;
import android.view.inputmethod.*;
import android.widget.EditText;
import java.lang.reflect.Field;
import org.json.JSONObject;

/** Runs the production activity/IME bridge in an isolated APK. The native stub
 * only counts real window touches; Rust editor behavior is covered by nextest. */
public final class SelectionBridgeTest extends Instrumentation {
    private static native int touches();
    private MainActivity activity;
    private EditText input;
    private InputConnection oldConnection;
    private String state(long revision, boolean secret) throws Exception {
        android.util.DisplayMetrics size = activity.getResources().getDisplayMetrics();
        return new JSONObject().put("id",1).put("revision",revision).put("request",0)
            .put("max_bytes",4096).put("text","one two three").put("start",4).put("end",7)
            .put("single_line",true).put("secret",secret).put("size",24)
            .put("rect",new org.json.JSONArray(new int[]{20,size.heightPixels/2,size.widthPixels-40,100}))
            .put("selection_rect",new org.json.JSONArray(new int[]{60,size.heightPixels/2+20,60,32})).toString();
    }
    private Object field(String name) throws Exception {
        Field f = MainActivity.class.getDeclaredField(name); f.setAccessible(true); return f.get(activity);
    }
    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Intent launch = new Intent().setComponent(new ComponentName(getTargetContext(), MainActivity.class))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            activity = (MainActivity)startActivitySync(launch);
            runOnMainSync(() -> { try {
                activity.syncInput(state(1,false)); input = (EditText)field("input");
                oldConnection = input.onCreateInputConnection(new EditorInfo());
            } catch (Exception e) { throw new RuntimeException(e); } });
            waitForIdleSync();
            runOnMainSync(activity::inputMenu);
            waitForIdleSync(); SystemClock.sleep(400);
            ActionMode menu = (ActionMode)field("inputMenu");
            check(menu != null && menu.getType() == ActionMode.TYPE_FLOATING, "Floating clipboard toolbar did not open");
            int before = touches(); long now = SystemClock.uptimeMillis();
            sendPointerSync(MotionEvent.obtain(now,now,MotionEvent.ACTION_DOWN,60,100,0));
            sendPointerSync(MotionEvent.obtain(now,now+30,MotionEvent.ACTION_UP,60,100,0));
            SystemClock.sleep(150);
            check(touches() >= before+2, "Toolbar swallowed the first underlying handle gesture");
            runOnMainSync(() -> menu.getMenu().performIdentifierAction(android.R.id.copy,0));
            runOnMainSync(() -> {
                ClipboardManager clipboard = (ClipboardManager)activity.getSystemService(Context.CLIPBOARD_SERVICE);
                check("two".contentEquals(clipboard.getPrimaryClip().getItemAt(0).getText()), "Copy used the wrong selection");
                activity.inputMenu();
            });
            runOnMainSync(() -> { try {
                ActionMode select = (ActionMode)field("inputMenu");
                select.getMenu().performIdentifierAction(android.R.id.selectAll,0);
                check(input.getSelectionStart()==0 && input.getSelectionEnd()==13, "Select All did not update the Editable");
                activity.syncInput(state(2,false));
                check(field("inputMenu")==null, "A new Rust revision must dismiss the old toolbar");
                check(!oldConnection.commitText("STALE",1), "Old IME revision was allowed to overwrite selection");
                check("one two three".contentEquals(input.getText()), "Stale callback changed text");
                activity.syncInput(state(3,true)); activity.inputMenu();
                ActionMode secret = (ActionMode)field("inputMenu");
                check(!secret.getMenu().findItem(android.R.id.copy).isVisible(), "Secret copy must stay disabled");
                check(!secret.getMenu().findItem(android.R.id.cut).isVisible(), "Secret cut must stay disabled");
                activity.syncInput("null");
                check(field("inputMenu")==null && input.getVisibility()==View.GONE, "Detached fields retain input UI");
            } catch (Exception e) { throw new RuntimeException(e); } });
            result.putString("stream","PASS: floating toolbar, first-touch pass-through, copy/select-all, stale IME and secret/detach fences\n");
            finish(Activity.RESULT_OK,result);
        } catch (Throwable error) {
            result.putString("stream","FAIL: " + android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED,result);
        }
    }
}
