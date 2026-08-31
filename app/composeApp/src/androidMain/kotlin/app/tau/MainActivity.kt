package app.tau

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        TauAndroidContext.initialize(applicationContext)
        PlatformServices.installCrashHandler()
        super.onCreate(savedInstanceState)
        setContent { TauApp() }
    }
}
