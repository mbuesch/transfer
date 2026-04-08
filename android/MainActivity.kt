package dev.dioxus.main;

import ch.bues.Transfer.BuildConfig;
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
import androidx.activity.result.ActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicReference

typealias BuildConfig = BuildConfig;

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
        if (intent == null) return
        val uris: List<Uri> = when (intent.action) {
            Intent.ACTION_SEND -> {
                val uri: Uri? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
                listOfNotNull(uri)
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                val list: List<Uri>? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
                }
                list ?: emptyList()
            }
            else -> return
        }
        if (uris.isEmpty()) return
        val activity = this
        Thread {
            for (uri in uris) {
                val path = copyUriToCache(activity, uri)
                if (path != null) {
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
        if (multicastLock == null || multicastLock?.isHeld == false) {
            multicastLock = wifiManager.createMulticastLock("transfer_multicast").also {
                it.setReferenceCounted(false)
                it.acquire()
            }
        }
        if (wifiLock == null || wifiLock?.isHeld == false) {
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

    companion object {
        @JvmStatic
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
            val latch = CountDownLatch(1)
            val resultUri = AtomicReference<Uri?>()

            Handler(Looper.getMainLooper()).post {
                val key = "rust_file_picker_${System.nanoTime()}"
                val launcher = activity.activityResultRegistry.register(
                    key,
                    ActivityResultContracts.StartActivityForResult()
                ) { result: ActivityResult ->
                    if (result.resultCode == Activity.RESULT_OK) {
                        resultUri.set(result.data?.data)
                    }
                    latch.countDown()
                }

                val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = "*/*"
                }
                try {
                    launcher.launch(intent)
                } catch (e: Exception) {
                    latch.countDown()
                }
            }

            latch.await()

            val uri = resultUri.get() ?: return null
            return copyUriToCache(activity, uri)
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
        fun pickFolder(): String? {
            val activity = instance ?: return null
            val latch = CountDownLatch(1)
            val resultUri = AtomicReference<Uri?>()

            Handler(Looper.getMainLooper()).post {
                val key = "rust_folder_picker_${System.nanoTime()}"
                val launcher = activity.activityResultRegistry.register(
                    key,
                    ActivityResultContracts.StartActivityForResult()
                ) { result: ActivityResult ->
                    if (result.resultCode == Activity.RESULT_OK) {
                        resultUri.set(result.data?.data)
                    }
                    latch.countDown()
                }
                val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                try {
                    launcher.launch(intent)
                } catch (e: Exception) {
                    latch.countDown()
                }
            }

            latch.await()
            val uri = resultUri.get() ?: return null
            return copyTreeUriToCache(activity, uri)
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
