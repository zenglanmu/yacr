# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# JNI contract: `libapp_android.so` calls these Activity instance methods by
# exact name and descriptor (`apps/app-android/src-tauri/src/saf.rs`). R8 must
# not rename, inline away, or remove them; without this block a release APK
# fails at runtime with NoSuchMethodError even though the debug build works.
-keepclassmembers class dev.yacr.app.MainActivity {
    public void documentLaunchOpen();
    public void documentLaunchSave(java.lang.String);
    public void documentReadUri(java.lang.String);
    public void documentWriteUri(java.lang.String, byte[]);
    public java.lang.Object[] documentSnapshot();
}
