package dev.dioxus.main

import ch.bues.Transfer.BuildConfig
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicReference

typealias BuildConfig = BuildConfig // re-export

class MainActivity : WryActivity() {

    private var multicastLock: WifiManager.MulticastLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    // Re-acquire locks whenever the wifi network becomes available so the driver
    // re-enables multicast/broadcast delivery for the new association.
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            releaseWifiLocks()
            acquireWifiLocks()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        instance = this
        // Acquire locks BEFORE the native library starts so that multicast/broadcast
        // delivery is enabled from the very first packet.
        acquireWifiLocks()
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .build()
        (getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager)
            .registerNetworkCallback(request, networkCallback)
        super.onCreate(savedInstanceState)
        handleShareIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShareIntent(intent)
    }

    private fun handleShareIntent(intent: Intent?) {
        val uris = getShareUris(intent)
        if (uris.isEmpty()) return

        Thread {
            for (uri in uris) {
                copyUriToCache(this, uri)?.let { path ->
                    synchronized(sharedFiles) {
                        sharedFiles.add(path)
                    }
                }
            }
        }.start()
    }

    override fun onDestroy() {
        (getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager)
            .unregisterNetworkCallback(networkCallback)
        releaseWifiLocks()
        if (instance === this) {
            instance = null
        }
        super.onDestroy()
    }

    private fun acquireWifiLocks() {
        val wifiManager = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            ?: return
        if (multicastLock?.isHeld != true) {
            multicastLock = wifiManager.createMulticastLock("transfer_multicast").also {
                it.setReferenceCounted(false)
                it.acquire()
            }
        }
        if (wifiLock?.isHeld != true) {
            @Suppress("DEPRECATION")
            val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                WifiManager.WIFI_MODE_FULL_LOW_LATENCY
            } else {
                WifiManager.WIFI_MODE_FULL_HIGH_PERF
            }
            wifiLock = wifiManager.createWifiLock(mode, "transfer_wifi").also {
                it.setReferenceCounted(false)
                it.acquire()
            }
        }
    }

    private fun releaseWifiLocks() {
        multicastLock?.takeIf { it.isHeld }?.release()
        wifiLock?.takeIf { it.isHeld }?.release()
    }

    private fun getShareUris(intent: Intent?): List<Uri> {
        if (intent == null) return emptyList()
        return when (intent.action) {
            Intent.ACTION_SEND -> intent.getParcelableExtraCompat(Intent.EXTRA_STREAM)
                ?.let(::listOf)
                .orEmpty()
            Intent.ACTION_SEND_MULTIPLE -> intent.getParcelableArrayListExtraCompat(Intent.EXTRA_STREAM)
                ?: emptyList()
            else -> emptyList()
        }
    }

    private fun Intent.getParcelableExtraCompat(name: String): Uri? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableExtra(name, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            getParcelableExtra(name) as? Uri
        }

    private fun Intent.getParcelableArrayListExtraCompat(name: String): ArrayList<Uri>? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableArrayListExtra(name, Uri::class.java)
        } else {
            @Suppress("DEPRECATION", "UNCHECKED_CAST")
            getParcelableArrayListExtra<Uri>(name) as? ArrayList<Uri>
        }

    companion object {
        @JvmStatic @Volatile
        var instance: MainActivity? = null

        @Volatile private var copyStatusMessage: String? = null

        private val sharedFiles = mutableListOf<String>()

        @JvmStatic
        fun getSharedFiles(): Array<String> {
            synchronized(sharedFiles) {
                return sharedFiles.toTypedArray()
            }
        }

        @JvmStatic
        fun clearSharedFiles() {
            synchronized(sharedFiles) {
                sharedFiles.clear()
            }
        }

        @JvmStatic
        fun getCopyStatus(): String? = copyStatusMessage

        @JvmStatic
        fun pickFile(): String? {
            val activity = instance ?: return null
            return awaitActivityResult(activity,
                createIntent = {
                    Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE)
                        type = "*/*"
                    }
                }
            ) { data -> data?.data?.let { copyUriToCache(activity, it) } }
        }

        @JvmStatic
        fun pickFolder(): String? {
            val activity = instance ?: return null
            return pickTreeUri(activity)?.let { copyTreeUriToCache(activity, it) }
        }

        @JvmStatic
        fun pickSaveFolder(): String? {
            val activity = instance ?: return null
            val uri = pickTreeUri(activity) ?: return null
            activity.contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            )
            return uri.toString()
        }

        private fun pickTreeUri(activity: ComponentActivity): Uri? =
            awaitActivityResult(activity, createIntent = { Intent(Intent.ACTION_OPEN_DOCUMENT_TREE) }) { data ->
                data?.data
            }

        private fun <T> awaitActivityResult(
            activity: ComponentActivity,
            createIntent: () -> Intent,
            handleResult: (Intent?) -> T?,
        ): T? {
            if (Looper.myLooper() == Looper.getMainLooper()) return null

            val latch = CountDownLatch(1)
            val resultRef = AtomicReference<T?>()
            val key = "rust_activity_${System.nanoTime()}"

            Handler(Looper.getMainLooper()).post {
                lateinit var launcher: ActivityResultLauncher<Intent>
                launcher = activity.activityResultRegistry.register(
                    key,
                    ActivityResultContracts.StartActivityForResult()
                ) { result: ActivityResult ->
                    if (result.resultCode == Activity.RESULT_OK) {
                        resultRef.set(handleResult(result.data))
                    }
                    latch.countDown()
                    launcher.unregister()
                }

                try {
                    launcher.launch(createIntent())
                } catch (_: Exception) {
                    latch.countDown()
                    launcher.unregister()
                }
            }

            latch.await()
            return resultRef.get()
        }

        private fun copyUriToCache(activity: Activity, uri: Uri): String? {
            copyStatusMessage = "Caching file..."
            try {
                val inputStream = activity.contentResolver.openInputStream(uri)
                    ?: return null
                val fileName = queryFileName(activity, uri) ?: "picked_file"
                val cacheFile = File(activity.cacheDir, fileName)
                inputStream.use { input ->
                    cacheFile.outputStream().use { output ->
                        input.copyTo(output)
                    }
                }
                return cacheFile.absolutePath
            } catch (e: Exception) {
                return null
            } finally {
                copyStatusMessage = null
            }
        }

        private fun queryFileName(activity: Activity, uri: Uri): String? {
            activity.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (idx >= 0) {
                        return cursor.getString(idx)
                    }
                }
            }
            return uri.lastPathSegment
        }

        @JvmStatic
        fun copyFolderToTree(treeUriString: String, sourcePath: String): Boolean {
            val activity = instance ?: return false
            return try {
                val treeUri = Uri.parse(treeUriString)
                val docId = DocumentsContract.getTreeDocumentId(treeUri)
                val rootDocUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
                copyDirectoryContentsToTree(activity, treeUri, rootDocUri, File(sourcePath))
            } catch (e: Exception) {
                false
            }
        }

        private fun copyDirectoryContentsToTree(
            activity: Activity,
            treeUri: Uri,
            parentDocUri: Uri,
            srcDir: File,
        ): Boolean {
            if (!srcDir.isDirectory) return false
            val files = srcDir.listFiles() ?: return false
            files.forEach { file ->
                if (file.isDirectory) {
                    val dirUri = findOrCreateDirectory(activity, treeUri, parentDocUri, file.name)
                        ?: return false
                    if (!copyDirectoryContentsToTree(activity, treeUri, dirUri, file)) return false
                } else {
                    if (!copyFileToTree(activity, treeUri, parentDocUri, file)) return false
                }
            }
            return true
        }

        private fun findOrCreateDirectory(
            activity: Activity,
            treeUri: Uri,
            parentDocUri: Uri,
            displayName: String,
        ): Uri? {
            queryChildDocument(activity, treeUri, parentDocUri, displayName)?.let { return it }
            return DocumentsContract.createDocument(
                activity.contentResolver,
                parentDocUri,
                DocumentsContract.Document.MIME_TYPE_DIR,
                displayName,
            )
        }

        private fun copyFileToTree(
            activity: Activity,
            treeUri: Uri,
            parentDocUri: Uri,
            srcFile: File,
        ): Boolean {
            val fileUri = queryChildDocument(activity, treeUri, parentDocUri, srcFile.name)
                ?: DocumentsContract.createDocument(
                    activity.contentResolver,
                    parentDocUri,
                    "application/octet-stream",
                    srcFile.name,
                )
                ?: return false
            activity.contentResolver.openOutputStream(fileUri, "w")?.use { output ->
                srcFile.inputStream().use { input -> input.copyTo(output) }
            } ?: return false
            return true
        }

        private fun queryChildDocument(
            activity: Activity,
            treeUri: Uri,
            parentDocUri: Uri,
            displayName: String,
        ): Uri? {
            val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(
                treeUri,
                DocumentsContract.getDocumentId(parentDocUri),
            )
            val projection = arrayOf(
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            )
            activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
                while (cursor.moveToNext()) {
                    if (cursor.getString(
                            cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                        ) == displayName
                    ) {
                        val documentId = cursor.getString(
                            cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
                        )
                        return DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId)
                    }
                }
            }
            return null
        }

        private fun copyTreeUriToCache(activity: Activity, treeUri: Uri): String? {
            copyStatusMessage = "Caching folder..."
            return try {
                val folderName = getFolderDisplayName(activity, treeUri) ?: "picked_folder"
                val destDir = File(activity.cacheDir, folderName)
                if (destDir.exists()) destDir.deleteRecursively()
                destDir.mkdirs()
                val docId = DocumentsContract.getTreeDocumentId(treeUri)
                val rootDocUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
                copyDocumentTree(activity, treeUri, rootDocUri, destDir)
                destDir.absolutePath
            } catch (e: Exception) {
                null
            } finally {
                copyStatusMessage = null
            }
        }

        private fun getFolderDisplayName(activity: Activity, treeUri: Uri): String? {
            val docId = DocumentsContract.getTreeDocumentId(treeUri) ?: return null
            val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
            activity.contentResolver.query(
                docUri,
                arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),
                null, null, null
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                    if (idx >= 0) return cursor.getString(idx)
                }
            }
            return null
        }

        private fun copyDocumentTree(activity: Activity, treeUri: Uri, docUri: Uri, destDir: File) {
            val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(
                treeUri,
                DocumentsContract.getDocumentId(docUri)
            )
            val projection = arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE,
            )
            activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
                while (cursor.moveToNext()) {
                    val childDocId = cursor.getString(
                        cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
                    )
                    val displayName = cursor.getString(
                        cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                    )
                    val mimeType = cursor.getString(
                        cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_MIME_TYPE)
                    )
                    val childDocUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, childDocId)
                    if (mimeType == DocumentsContract.Document.MIME_TYPE_DIR) {
                        val subDir = File(destDir, displayName)
                        subDir.mkdirs()
                        copyDocumentTree(activity, treeUri, childDocUri, subDir)
                    } else {
                        val destFile = File(destDir, displayName)
                        activity.contentResolver.openInputStream(childDocUri)?.use { input ->
                            destFile.outputStream().use { output ->
                                input.copyTo(output)
                            }
                        }
                    }
                }
            }
        }
    }
}
