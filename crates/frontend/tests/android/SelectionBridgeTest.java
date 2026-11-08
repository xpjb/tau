package app.tau.rust;

import android.app.*;
import android.content.*;
import android.os.*;
import android.view.*;
import android.view.inputmethod.*;
import android.widget.EditText;
import io.github.xpjb.chad.ChadActivity;
import java.lang.reflect.Field;

/** Runs Tau's production ChadActivity subclass in an isolated APK. The native
 * stub only counts real window touches; Rust editor behavior is covered by nextest. */
public final class SelectionBridgeTest extends Instrumentation {
    private static native int touches();
    private MainActivity activity;
    private EditText input;
    private InputConnection oldConnection;
    private void state(long generation, boolean secret) {
        android.util.DisplayMetrics size = activity.getResources().getDisplayMetrics();
        activity.chadSetTextInput(1, generation, 0, "one two three", 4, 7,
            20, size.heightPixels / 2.0f, size.widthPixels - 40, 100,
            60, size.heightPixels / 2.0f + 20, 60, 32,
            false, secret, true);
    }
    private Object field(String name) throws Exception {
        Field field = ChadActivity.class.getDeclaredField(name);
        field.setAccessible(true);
        return field.get(activity);
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
            runOnMainSync(() -> {
                state(1, false);
                try {
                    input = (EditText)field("input");
                    oldConnection = input.onCreateInputConnection(new EditorInfo());
                } catch (Exception error) { throw new RuntimeException(error); }
            });
            waitForIdleSync(); SystemClock.sleep(300);
            android.graphics.Bitmap beforeMenu = getUiAutomation().takeScreenshot();
            runOnMainSync(activity::chadShowTextMenu);
            waitForIdleSync(); SystemClock.sleep(400);
            ActionMode menu = (ActionMode)field("inputMenu");
            check(menu != null && menu.getType() == ActionMode.TYPE_FLOATING, "Floating clipboard toolbar did not open");
            android.graphics.Bitmap shownMenu = getUiAutomation().takeScreenshot();
            int changed = 0;
            for (int y=beforeMenu.getHeight()/4; y<beforeMenu.getHeight()*3/4; y++)
                for (int x=0; x<beforeMenu.getWidth(); x++)
                    if (beforeMenu.getPixel(x,y)!=shownMenu.getPixel(x,y)) changed++;
            check(changed>500, "Toolbar exists but is not visibly painted over the NativeActivity surface");
            beforeMenu.recycle(); shownMenu.recycle();
            int before = touches(); long now = SystemClock.uptimeMillis();
            float touchY = activity.getResources().getDisplayMetrics().heightPixels - 200;
            sendPointerSync(MotionEvent.obtain(now,now,MotionEvent.ACTION_DOWN,60,touchY,0));
            sendPointerSync(MotionEvent.obtain(now,now+30,MotionEvent.ACTION_UP,60,touchY,0));
            SystemClock.sleep(150);
            check(touches() >= before+2, "Toolbar swallowed the first underlying handle gesture");
            runOnMainSync(() -> menu.getMenu().performIdentifierAction(android.R.id.copy,0));
            runOnMainSync(() -> {
                ClipboardManager clipboard = (ClipboardManager)activity.getSystemService(Context.CLIPBOARD_SERVICE);
                check("two".contentEquals(clipboard.getPrimaryClip().getItemAt(0).getText()), "Copy used the wrong selection");
                activity.chadShowTextMenu();
            });
            runOnMainSync(() -> { try {
                ActionMode select = (ActionMode)field("inputMenu");
                select.getMenu().performIdentifierAction(android.R.id.selectAll,0);
                check(input.getSelectionStart()==0 && input.getSelectionEnd()==13, "Select All did not update the Editable");
                state(2,false);
                check(field("inputMenu")==null, "A new Rust generation must dismiss the old toolbar");
                check(!oldConnection.commitText("STALE",1), "Old IME generation was allowed to overwrite selection");
                check("one two three".contentEquals(input.getText()), "Stale callback changed text");
                state(3,true); activity.chadShowTextMenu();
                ActionMode secret = (ActionMode)field("inputMenu");
                check(!secret.getMenu().findItem(android.R.id.copy).isVisible(), "Secret copy must stay disabled");
                check(!secret.getMenu().findItem(android.R.id.cut).isVisible(), "Secret cut must stay disabled");
                activity.chadClearTextInput();
                check(field("inputMenu")==null && input.getVisibility()==View.GONE, "Detached fields retain input UI");
            } catch (Exception error) { throw new RuntimeException(error); } });
            result.putString("stream","PASS: inherited floating toolbar, first-touch pass-through, copy/select-all, stale IME and secret/detach fences\n");
            finish(Activity.RESULT_OK,result);
        } catch (Throwable error) {
            result.putString("stream","FAIL: " + android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED,result);
        }
    }
}
