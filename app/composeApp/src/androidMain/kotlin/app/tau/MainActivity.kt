package app.tau

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import kotlinx.coroutines.Dispatchers

class MainActivity : ComponentActivity() {
    private lateinit var controller: TauController
    private val filePicker = registerForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments(),
        TauAndroidContext::completeFileSelection,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        TauAndroidContext.initialize(applicationContext, filePicker)
        PlatformServices.installCrashHandler()
        super.onCreate(savedInstanceState)
        controller = TauController(Dispatchers.Main.immediate)
        setContent { TauApp(controller) }
    }
}
