#[cfg(not(target_os = "android"))]
pub fn get_device_name() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string())
}

#[cfg(target_os = "android")]
pub fn get_device_name() -> String {
    (|| -> Option<String> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<String>, jni::errors::Error> {
            let activity = unsafe { jni::objects::JObject::from_raw(env, ctx.context().cast()) };

            // Try Settings.Global.DEVICE_NAME first
            let settings_global =
                env.find_class(jni::jni_str!("android/provider/Settings$Global"))?;
            let content_resolver = env
                .call_method(
                    &activity,
                    jni::jni_str!("getContentResolver"),
                    jni::jni_sig!("()Landroid/content/ContentResolver;"),
                    &[],
                )?
                .l()?;
            let key = env.new_string("device_name")?;
            let result = env
                .call_static_method(
                    settings_global,
                    jni::jni_str!("getString"),
                    jni::jni_sig!(
                        "(Landroid/content/ContentResolver;Ljava/lang/String;)Ljava/lang/String;"
                    ),
                    &[(&content_resolver).into(), (&key).into()],
                )?
                .l()?;
            if !result.is_null() {
                let jstr = env.cast_local::<jni::objects::JString>(result)?;
                let name = jstr.try_to_string(env)?;
                if !name.is_empty() {
                    return Ok(Some(name));
                }
            }

            // Fall back to Build.MODEL
            let build_class = env.find_class(jni::jni_str!("android/os/Build"))?;
            let model_field = env
                .get_static_field(
                    build_class,
                    jni::jni_str!("MODEL"),
                    jni::jni_sig!("Ljava/lang/String;"),
                )?
                .l()?;
            let jstr = env.cast_local::<jni::objects::JString>(model_field)?;
            let model = jstr.try_to_string(env)?;
            Ok(Some(model))
        })
        .ok()?
    })()
    .unwrap_or_else(|| "Android".to_string())
}
