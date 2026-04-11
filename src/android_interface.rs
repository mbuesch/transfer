use crate::l10n::Language;
use anyhow as ah;
use std::path::PathBuf;

/// Retrieve file paths shared via Android's share intent (ACTION_SEND / ACTION_SEND_MULTIPLE).
pub fn android_get_shared_files() -> Vec<PathBuf> {
    (|| -> Option<Vec<PathBuf>> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(
            |env| -> Result<Option<Vec<PathBuf>>, ::jni::errors::Error> {
                let activity =
                    unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
                let class = env.get_object_class(&activity)?;
                let result = env.call_static_method(
                    &class,
                    ::jni::jni_str!("getSharedFiles"),
                    ::jni::jni_sig!("()[Ljava/lang/String;"),
                    &[],
                )?;
                let jobj = result.l()?;
                if jobj.is_null() {
                    return Ok(Some(vec![]));
                }
                let array =
                    env.cast_local::<::jni::objects::JObjectArray<::jni::objects::JString>>(jobj)?;
                let len = array.len(env)?;
                let mut paths = vec![];
                for i in 0..len {
                    let elem: ::jni::objects::JString = array.get_element(env, i)?;
                    if !elem.is_null() {
                        let s = elem.try_to_string(env)?;
                        paths.push(PathBuf::from(s));
                    }
                }
                // Clear the shared files after retrieval
                let _ = env.call_static_method(
                    &class,
                    ::jni::jni_str!("clearSharedFiles"),
                    ::jni::jni_sig!("()V"),
                    &[],
                );
                Ok(Some(paths))
            },
        )
        .ok()?
    })()
    .unwrap_or_default()
}

pub fn android_get_copy_status() -> Option<String> {
    (|| -> Option<String> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<String>, ::jni::errors::Error> {
            let activity = unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
            let class = env.get_object_class(&activity)?;
            let result = env.call_static_method(
                &class,
                ::jni::jni_str!("getCopyStatus"),
                ::jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?;
            let jobj = result.l()?;
            if jobj.is_null() {
                return Ok(None);
            }
            let jstr = env.cast_local::<::jni::objects::JString>(jobj)?;
            let s = jstr.try_to_string(env)?;
            Ok(Some(s))
        })
        .ok()?
    })()
}

pub async fn android_pick_file_to_send(_lang: Language) -> Option<PathBuf> {
    tokio::task::spawn_blocking(|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<PathBuf>, ::jni::errors::Error> {
            let activity = unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
            let class = env.get_object_class(&activity)?;
            let result = env.call_static_method(
                &class,
                ::jni::jni_str!("pickFile"),
                ::jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?;
            let jobj = result.l()?;
            if jobj.is_null() {
                return Ok(None);
            }
            let jstr = env.cast_local::<::jni::objects::JString>(jobj)?;
            let path_str = jstr.try_to_string(env)?;
            Ok(Some(PathBuf::from(path_str)))
        })
        .ok()?
    })
    .await
    .ok()?
}

pub async fn android_pick_folder_to_send(_lang: Language) -> Option<PathBuf> {
    tokio::task::spawn_blocking(|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<PathBuf>, ::jni::errors::Error> {
            let activity = unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
            let class = env.get_object_class(&activity)?;
            let result = env.call_static_method(
                &class,
                ::jni::jni_str!("pickFolder"),
                ::jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?;
            let jobj = result.l()?;
            if jobj.is_null() {
                return Ok(None);
            }
            let jstr = env.cast_local::<::jni::objects::JString>(jobj)?;
            let path_str = jstr.try_to_string(env)?;
            Ok(Some(PathBuf::from(path_str)))
        })
        .ok()?
    })
    .await
    .ok()?
}

pub async fn android_pick_save_folder(_lang: Language) -> Option<PathBuf> {
    tokio::task::spawn_blocking(|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<PathBuf>, ::jni::errors::Error> {
            let activity = unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
            let class = env.get_object_class(&activity)?;
            let result = env.call_static_method(
                &class,
                ::jni::jni_str!("pickSaveFolder"),
                ::jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?;
            let jobj = result.l()?;
            if jobj.is_null() {
                return Ok(None);
            }
            let jstr = env.cast_local::<::jni::objects::JString>(jobj)?;
            let path_str = jstr.try_to_string(env)?;
            Ok(Some(PathBuf::from(path_str)))
        })
        .ok()?
    })
    .await
    .ok()?
}

pub fn android_copy_folder_to_tree(
    tree_uri: &str,
    source_path: &std::path::Path,
) -> ah::Result<()> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
    let tree_uri = tree_uri.to_string();
    let source = source_path.to_string_lossy().to_string();
    vm.attach_current_thread(|env| -> Result<(), ::jni::errors::Error> {
        let activity = unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
        let class = env.get_object_class(&activity)?;
        let tree_jstr = env.new_string(tree_uri)?;
        let src_jstr = env.new_string(source)?;
        let tree_obj = ::jni::objects::JObject::from(tree_jstr);
        let src_obj = ::jni::objects::JObject::from(src_jstr);
        let result = env.call_static_method(
            &class,
            ::jni::jni_str!("copyFolderToTree"),
            ::jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Z"),
            &[
                ::jni::objects::JValue::Object(&tree_obj),
                ::jni::objects::JValue::Object(&src_obj),
            ],
        )?;
        if !result.z()? {
            return Err(::jni::errors::Error::JavaException);
        }
        Ok(())
    })?;
    Ok(())
}
