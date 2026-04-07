# Keep all methods in MainActivity that are called from Rust via JNI by name.
# R8 (isMinifyEnabled = true in release builds) has no visibility into the JNI
# call sites in native code, so it would otherwise freely rename or remove
# these public static methods, causing GetStaticMethodID to return NULL and the
# JNI call to fail silently (pick-file / pick-folder dialogs never open).
-keepclassmembers class dev.dioxus.main.MainActivity {
    public static java.lang.String   pickFile();
    public static java.lang.String   pickFolder();
    public static java.lang.String[] getSharedFiles();
    public static void               clearSharedFiles();
    public static java.lang.String   getCopyStatus();
}
