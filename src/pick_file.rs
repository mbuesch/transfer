use crate::l10n::Language;
use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
pub async fn pick_file_to_send(lang: Language) -> Option<PathBuf> {
    let fh = rfd::AsyncFileDialog::new()
        .set_title(lang.select_file_title())
        .pick_file()
        .await;
    fh.map(|h| PathBuf::from(h.path()))
}

#[cfg(target_os = "android")]
pub async fn pick_file_to_send(_lang: Language) -> Option<PathBuf> {
    tokio::task::spawn_blocking(|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<PathBuf>, jni::errors::Error> {
            let activity = unsafe { jni::objects::JObject::from_raw(env, ctx.context().cast()) };
            let class = env.get_object_class(&activity)?;
            let result = env.call_static_method(
                &class,
                jni::jni_str!("pickFile"),
                jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?;
            let jobj = result.l()?;
            if jobj.is_null() {
                return Ok(None);
            }
            let jstr = env.cast_local::<jni::objects::JString>(jobj)?;
            let path_str = jstr.try_to_string(env)?;
            Ok(Some(PathBuf::from(path_str)))
        })
        .ok()?
    })
    .await
    .ok()?
}

#[cfg(not(target_os = "android"))]
pub async fn pick_save_folder(lang: Language) -> Option<PathBuf> {
    let fh = rfd::AsyncFileDialog::new()
        .set_title(lang.select_folder_title())
        .pick_folder()
        .await;
    fh.map(|h| PathBuf::from(h.path()))
}

#[cfg(target_os = "android")]
pub async fn pick_save_folder(_lang: Language) -> Option<PathBuf> {
    tokio::task::spawn_blocking(|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<PathBuf>, jni::errors::Error> {
            let activity = unsafe { jni::objects::JObject::from_raw(env, ctx.context().cast()) };
            let class = env.get_object_class(&activity)?;
            let result = env.call_static_method(
                &class,
                jni::jni_str!("pickFolder"),
                jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?;
            let jobj = result.l()?;
            if jobj.is_null() {
                return Ok(None);
            }
            let jstr = env.cast_local::<jni::objects::JString>(jobj)?;
            let path_str = jstr.try_to_string(env)?;
            Ok(Some(PathBuf::from(path_str)))
        })
        .ok()?
    })
    .await
    .ok()?
}
