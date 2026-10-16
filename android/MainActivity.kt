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
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
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
        cleanupStaleCacheImports(this)
        // Acquire locks BEFORE the native library starts so that multicast/broadcast
        // delivery is enabled from the very first packet.
        acquireWifiLocks()
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .build()
        (getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager)
            .registerNetworkCallback(request, networkCallback)
        super.onCreate(savedInstanceState)
        reattachPendingPicks(this)
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
        if (isFinishing) {
            cancelPendingPicks()
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

        private const val CACHE_IMPORT_PREFIX = "import-"
        private const val MAX_CACHE_NAME_BYTES = 200
        private const val MAX_CACHE_EXTENSION_CHARS = 16

        private val staleCacheCleanupStarted = AtomicBoolean(false)

        private val copyStatusMessage = java.util.concurrent.atomic.AtomicReference<String?>(null)

        private val sharedFiles = mutableListOf<String>()

        @JvmStatic
        fun getSharedFiles(): Array<String> {
            // Drain atomically so files copied concurrently are never lost.
            synchronized(sharedFiles) {
                val result = sharedFiles.toTypedArray()
                sharedFiles.clear()
                return result
            }
        }

        @JvmStatic
        fun getCopyStatus(): String? = copyStatusMessage.get()

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

            val pending = PendingPick(
                onResult = { result ->
                    if (result.resultCode == Activity.RESULT_OK) {
                        resultRef.set(handleResult(result.data))
                    }
                    latch.countDown()
                },
                onCancel = { latch.countDown() },
            )
            pendingPicks[key] = pending

            Handler(Looper.getMainLooper()).post {
                val launcher = registerPending(activity, key, pending)
                try {
                    launcher.launch(createIntent())
                } catch (_: Exception) {
                    pendingPicks.remove(key)
                    launcher.unregister()
                    latch.countDown()
                }
            }

            latch.await()
            return resultRef.get()
        }

        private class PendingPick(
            val onResult: (ActivityResult) -> Unit,
            val onCancel: () -> Unit,
        )

        private val pendingPicks = ConcurrentHashMap<String, PendingPick>()

        private fun registerPending(
            activity: ComponentActivity,
            key: String,
            pending: PendingPick,
        ): ActivityResultLauncher<Intent> {
            lateinit var launcher: ActivityResultLauncher<Intent>
            launcher = activity.activityResultRegistry.register(
                key,
                ActivityResultContracts.StartActivityForResult()
            ) { result: ActivityResult ->
                pendingPicks.remove(key)
                launcher.unregister()
                pending.onResult(result)
            }
            return launcher
        }

        // Re-attach in-flight pickers to a recreated Activity so the registry
        // can deliver the result that arrived while the old one was destroyed.
        private fun reattachPendingPicks(activity: ComponentActivity) {
            for ((key, pending) in pendingPicks) {
                registerPending(activity, key, pending)
            }
        }

        private fun cancelPendingPicks() {
            val all = pendingPicks.values.toList()
            pendingPicks.clear()
            all.forEach { it.onCancel() }
        }

        private fun copyUriToCache(activity: Activity, uri: Uri): String? {
            copyStatusMessage.set("Caching file...")
            var importDir: File? = null
            try {
                val inputStream = activity.contentResolver.openInputStream(uri)
                    ?: throw IOException("Failed to open selected file")
                inputStream.use { input ->
                    val dir = createCacheImportDirectory(activity)
                    importDir = dir
                    val cacheFile = uniqueCacheChild(
                        dir,
                        queryFileName(activity, uri),
                        "picked_file",
                    )
                    cacheFile.outputStream().use { output ->
                        input.copyTo(output)
                    }
                    return cacheFile.absolutePath
                }
            } catch (e: Exception) {
                discardFailedCacheImport(importDir, e, "file")
                return null
            } finally {
                copyStatusMessage.set(null)
            }
        }

        // Every import lives in its own directory below the cache directory. Nothing
        // removes it while the app runs, because a transfer may still read from it.
        // Leftovers of earlier app runs are removed once per process start.
        private fun cleanupStaleCacheImports(activity: Activity) {
            if (!staleCacheCleanupStarted.compareAndSet(false, true)) return
            val stale = activity.cacheDir.listFiles { file ->
                file.isDirectory && file.name.startsWith(CACHE_IMPORT_PREFIX)
            }
            if (stale.isNullOrEmpty()) return
            Thread {
                for (dir in stale) {
                    if (!dir.deleteRecursively()) {
                        Log.w("MainActivity", "Failed to remove stale cache import")
                    }
                }
            }.start()
        }

        private fun createCacheImportDirectory(activity: Activity): File {
            val importDir = File(activity.cacheDir, "$CACHE_IMPORT_PREFIX${UUID.randomUUID()}")
            if (!importDir.mkdir()) {
                throw IOException("Failed to create cache import directory")
            }
            return importDir
        }

        private fun uniqueCacheChild(parent: File, displayName: String?, fallback: String): File {
            val safeName = (displayName ?: fallback)
                .replace('/', '_')
                .replace('\\', '_')
                .replace('\u0000', '_')
                .takeUnless { it.isBlank() || it == "." || it == ".." }
                ?.let(::limitFileNameLength)
                ?: fallback
            val initial = File(parent, safeName)
            if (!initial.exists()) return initial

            val dotPos = safeName.lastIndexOf('.')
            val baseName = if (dotPos > 0) safeName.substring(0, dotPos) else safeName
            val extension = if (dotPos > 0) safeName.substring(dotPos) else ""
            for (i in 1..10000) {
                val candidate = File(parent, "$baseName ($i)$extension")
                if (!candidate.exists()) return candidate
            }
            throw IOException("Failed to create a unique cache entry name")
        }

        // File system limit is 255 bytes. Keep room for the " (N)" collision suffix.
        private fun limitFileNameLength(name: String): String {
            if (name.toByteArray().size <= MAX_CACHE_NAME_BYTES) return name
            val dotPos = name.lastIndexOf('.')
            val extension = if (dotPos > 0 && name.length - dotPos <= MAX_CACHE_EXTENSION_CHARS) {
                name.substring(dotPos)
            } else {
                ""
            }
            var base = name.substring(0, name.length - extension.length)
            val extensionBytes = extension.toByteArray().size
            while (base.isNotEmpty() && base.toByteArray().size + extensionBytes > MAX_CACHE_NAME_BYTES) {
                base = base.dropLast(1)
            }
            if (base.isNotEmpty() && base.last().isHighSurrogate()) {
                base = base.dropLast(1)
            }
            return base + extension
        }

        private fun discardFailedCacheImport(importDir: File?, error: Exception, kind: String) {
            Log.e("MainActivity", "Failed to cache $kind import", error)
            if (importDir != null && importDir.exists() && !importDir.deleteRecursively()) {
                Log.w("MainActivity", "Failed to remove partial cache import")
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

        private fun findAvailableFileName(
            activity: Activity,
            treeUri: Uri,
            parentDocUri: Uri,
            displayName: String,
        ): String? {
            if (queryChildDocument(activity, treeUri, parentDocUri, displayName) == null) {
                return displayName
            }
            val dotPos = displayName.lastIndexOf('.')
            val baseName: String
            val extension: String
            if (dotPos >= 0) {
                baseName = displayName.substring(0, dotPos)
                extension = displayName.substring(dotPos)
            } else {
                baseName = displayName
                extension = ""
            }
            for (i in 1..10000) {
                val candidate = "$baseName ($i)$extension"
                if (queryChildDocument(activity, treeUri, parentDocUri, candidate) == null) {
                    return candidate
                }
            }
            return null
        }

        private fun copyFileToTree(
            activity: Activity,
            treeUri: Uri,
            parentDocUri: Uri,
            srcFile: File,
        ): Boolean {
            val finalName = findAvailableFileName(activity, treeUri, parentDocUri, srcFile.name)
                ?: return false
            val fileUri = DocumentsContract.createDocument(
                activity.contentResolver,
                parentDocUri,
                "application/octet-stream",
                finalName,
            ) ?: return false
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
            copyStatusMessage.set("Caching folder...")
            var importDir: File? = null
            return try {
                importDir = createCacheImportDirectory(activity)
                val destDir = uniqueCacheChild(
                    importDir,
                    getFolderDisplayName(activity, treeUri),
                    "picked_folder",
                )
                if (!destDir.mkdir()) {
                    throw IOException("Failed to create cached folder")
                }
                val docId = DocumentsContract.getTreeDocumentId(treeUri)
                val rootDocUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
                copyDocumentTree(activity, treeUri, rootDocUri, destDir)
                destDir.absolutePath
            } catch (e: Exception) {
                discardFailedCacheImport(importDir, e, "folder")
                null
            } finally {
                copyStatusMessage.set(null)
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
            val cursor = activity.contentResolver.query(childrenUri, projection, null, null, null)
                ?: throw IOException("Failed to list selected folder")
            cursor.use {
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
                        val subDir = uniqueCacheChild(destDir, displayName, "picked_folder")
                        if (!subDir.mkdir()) {
                            throw IOException("Failed to create cached subfolder")
                        }
                        copyDocumentTree(activity, treeUri, childDocUri, subDir)
                    } else {
                        val destFile = uniqueCacheChild(destDir, displayName, "picked_file")
                        val input = activity.contentResolver.openInputStream(childDocUri)
                            ?: throw IOException("Failed to open file in selected folder")
                        input.use {
                            destFile.outputStream().use { output ->
                                it.copyTo(output)
                            }
                        }
                    }
                }
            }
        }
    }
}
