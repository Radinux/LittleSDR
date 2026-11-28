//! JNI surface: app.littlesdr.engine.NativeBridge (Kotlin) calls these. Every call goes to
//! the service; the display ones to the renderer.

use super::{audio, render};
use crate::service::{service, Service};
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::sys::{jboolean, jbyteArray, jdouble, jdoubleArray, jfloat, jint, jintArray, jobjectArray, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

fn s(env: &mut JNIEnv, j: &JString) -> String {
    if j.is_null() {
        return String::new();
    }
    env.get_string(j).map(|v| v.into()).unwrap_or_default()
}

fn js(env: &mut JNIEnv, v: &str) -> jstring {
    env.new_string(v).map(|o| o.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn svc() -> Option<&'static Service> {
    service()
}

// ---------------------------------------------------------------- lifecycle

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_nativeInit(mut env: JNIEnv, _: JClass, root: JString, _cache: JString, fft: jint) -> jboolean {
    let root = s(&mut env, &root);
    super::log(&format!("init root={root} fft={fft}"));
    Service::init(&root, fft.max(0) as usize);
    JNI_TRUE
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setUsbDevice(_: JNIEnv, _: JClass, fd: jint, vid: jint, pid: jint) {
    super::log(&format!("USB device fd={fd} {vid:04x}:{pid:04x}"));
    if let Some(v) = svc() {
        v.set_usb_device(Some(fd));
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_clearUsbDevice(_: JNIEnv, _: JClass) {
    if let Some(v) = svc() {
        v.set_usb_device(None);
    }
}

// ---------------------------------------------------------------- sources

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_sourceNames(mut env: JNIEnv, _: JClass) -> jobjectArray {
    let names = svc().map(|v| v.source_names()).unwrap_or_default();
    let Ok(arr) = env.new_object_array(names.len() as i32, "java/lang/String", JObject::null()) else { return std::ptr::null_mut() };
    for (i, n) in names.iter().enumerate() {
        if let Ok(js) = env.new_string(n) {
            let _ = env.set_object_array_element(&arr, i as i32, js);
        }
    }
    arr.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_selectedSource(mut env: JNIEnv, _: JClass) -> jstring {
    let v = svc().map(|v| v.selected_source()).unwrap_or_default();
    js(&mut env, &v)
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_selectSource(mut env: JNIEnv, _: JClass, name: JString) {
    let n = s(&mut env, &name);
    if let Some(v) = svc() {
        v.select_source(&n);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_sourceError(mut env: JNIEnv, _: JClass) -> jstring {
    let v = svc().map(|v| v.source_error()).unwrap_or_default();
    js(&mut env, &v)
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_sourceMenu(mut env: JNIEnv, _: JClass, id: JString, _t: jint, b: jboolean, i: jint, f: jfloat, text: JString) -> jstring {
    let (id, text) = (s(&mut env, &id), s(&mut env, &text));
    let v = svc().map(|v| v.source_menu(&id, b != JNI_FALSE, i, f, &text)).unwrap_or_else(|| "[]".into());
    js(&mut env, &v)
}

// ---------------------------------------------------------------- playback / tuning

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_start(_: JNIEnv, _: JClass) -> jboolean {
    let Some(v) = svc() else { return JNI_FALSE };
    match v.try_start() {
        Ok(()) => {
            audio::set_device(v.option("audio", "device") as i32);
            audio::start(v.audio());
            JNI_TRUE
        }
        Err(e) => {
            super::log(&format!("start: {e}"));
            JNI_FALSE
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_stop(_: JNIEnv, _: JClass) {
    audio::stop();
    if let Some(v) = svc() {
        v.stop();
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_tune(_: JNIEnv, _: JClass, hz: jdouble) {
    if let Some(v) = svc() {
        v.tune(hz);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_tuneCenter(_: JNIEnv, _: JClass, hz: jdouble) {
    if let Some(v) = svc() {
        v.tune_center(hz);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_state(env: JNIEnv, _: JClass) -> jdoubleArray {
    let st = svc().map(|v| v.state()).unwrap_or([0.0; 11]);
    let Ok(arr) = env.new_double_array(11) else { return std::ptr::null_mut() };
    let _ = env.set_double_array_region(&arr, 0, &st);
    arr.into_raw()
}

// ---------------------------------------------------------------- demodulator

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setMode(_: JNIEnv, _: JClass, m: jint) {
    if let Some(v) = svc() {
        v.set_mode(m);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setBandwidth(_: JNIEnv, _: JClass, hz: jfloat) {
    if let Some(v) = svc() {
        v.set_bandwidth(hz);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setSquelch(_: JNIEnv, _: JClass, on: jboolean, level: jfloat) {
    if let Some(v) = svc() {
        v.set_squelch(on != JNI_FALSE, level);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setVolume(_: JNIEnv, _: JClass, vol: jfloat) {
    if let Some(v) = svc() {
        v.set_volume(vol);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_estimateBandwidth(_: JNIEnv, _: JClass, min: jdouble, max: jdouble) -> jfloat {
    svc().map_or(0.0, |v| v.estimate_bandwidth(min, max))
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_options(mut env: JNIEnv, _: JClass) -> jstring {
    let v = svc().map(|v| v.options_json()).unwrap_or_else(|| "{}".into());
    js(&mut env, &v)
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setOption(mut env: JNIEnv, _: JClass, key: JString, value: jdouble) {
    let k = s(&mut env, &key);
    if let Some(v) = svc() {
        v.set_option(&k, value);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setOptionString(mut env: JNIEnv, _: JClass, key: JString, value: JString) {
    let (k, val) = (s(&mut env, &key), s(&mut env, &value));
    if let Some(v) = svc() {
        v.set_option_string(&k, &val);
    }
}

// ---------------------------------------------------------------- decoders

macro_rules! json_get {
    ($fname:ident, $method:ident) => {
        #[no_mangle]
        pub extern "system" fn $fname(mut env: JNIEnv, _: JClass) -> jstring {
            let v = svc().map(|v| v.$method()).unwrap_or_else(|| "{}".into());
            js(&mut env, &v)
        }
    };
}

macro_rules! json_set {
    ($fname:ident, $method:ident) => {
        #[no_mangle]
        pub extern "system" fn $fname(mut env: JNIEnv, _: JClass, key: JString, value: jdouble) {
            let k = s(&mut env, &key);
            if let Some(v) = svc() {
                v.$method(&k, value);
            }
        }
    };
}

json_get!(Java_app_littlesdr_engine_NativeBridge_ft8Json, ft8_json);
json_set!(Java_app_littlesdr_engine_NativeBridge_ft8Set, ft8_set);
json_get!(Java_app_littlesdr_engine_NativeBridge_cwJson, cw_json);
json_set!(Java_app_littlesdr_engine_NativeBridge_cwSet, cw_set);
json_get!(Java_app_littlesdr_engine_NativeBridge_recJson, rec_json);
json_set!(Java_app_littlesdr_engine_NativeBridge_recSet, rec_set);
json_get!(Java_app_littlesdr_engine_NativeBridge_sstvJson, sstv_json);
json_set!(Java_app_littlesdr_engine_NativeBridge_sstvSet, sstv_set);
json_get!(Java_app_littlesdr_engine_NativeBridge_rdsJson, rds_json);

json_get!(Java_app_littlesdr_engine_NativeBridge_digitalJson, digital_json);
json_get!(Java_app_littlesdr_engine_NativeBridge_lrptJson, lrpt_json);
json_get!(Java_app_littlesdr_engine_NativeBridge_lrptSave, lrpt_save);
json_set!(Java_app_littlesdr_engine_NativeBridge_lrptSet, lrpt_set);
json_get!(Java_app_littlesdr_engine_NativeBridge_adsbJson, adsb_json);
json_get!(Java_app_littlesdr_engine_NativeBridge_vorJson, vor_json);
json_set!(Java_app_littlesdr_engine_NativeBridge_adsbSet, adsb_set);

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_dabSelect(mut env: JNIEnv, _: JClass, what: JString) {
    let w = s(&mut env, &what);
    if let Some(v) = svc() {
        v.dab_select(&w);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_dabSlide(env: JNIEnv, _: JClass) -> jbyteArray {
    let b = svc().map(|v| v.dab_slide()).unwrap_or_default();
    env.byte_array_from_slice(&b).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_lrptImage(env: JNIEnv, _: JClass) -> jintArray {
    let px = svc().map(|v| v.lrpt_image()).unwrap_or_else(|| vec![0, 0]);
    let Ok(arr) = env.new_int_array(px.len() as i32) else { return std::ptr::null_mut() };
    let _ = env.set_int_array_region(&arr, 0, &px);
    arr.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_kiwiSetSite(mut env: JNIEnv, _: JClass, host: JString, _loc: JString) {
    let h = s(&mut env, &host);
    if let Some(v) = svc() {
        v.kiwi_set_site(&h);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_sstvImage(env: JNIEnv, _: JClass, which: jint) -> jintArray {
    let px = svc().map(|v| v.sstv_image(which)).unwrap_or_else(|| vec![0, 0]);
    let Ok(arr) = env.new_int_array(px.len() as i32) else { return std::ptr::null_mut() };
    let _ = env.set_int_array_region(&arr, 0, &px);
    arr.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_ctyLatLon(mut env: JNIEnv, _: JClass, calls: JObjectArray) -> jdoubleArray {
    let n = env.get_array_length(&calls).unwrap_or(0);
    let mut out = Vec::with_capacity(2 * n as usize);
    for i in 0..n {
        let call = env.get_object_array_element(&calls, i).map(JString::from).map(|j| s(&mut env, &j)).unwrap_or_default();
        let (lat, lon) = crate::service::cty::lookup(&call).unwrap_or((f64::NAN, f64::NAN));
        out.push(lat);
        out.push(lon);
    }
    let Ok(arr) = env.new_double_array(out.len() as i32) else { return std::ptr::null_mut() };
    let _ = env.set_double_array_region(&arr, 0, &out);
    arr.into_raw()
}

// ---------------------------------------------------------------- display

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setView(_: JNIEnv, _: JClass, bw: jdouble, offset: jdouble) {
    if let Some(v) = svc() {
        v.set_view(bw, offset);
    }
    render::update(|p| {
        p.view_bandwidth = bw;
        p.view_offset = offset;
    });
}

fn argb(c: jint) -> [f32; 4] {
    let c = c as u32;
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0, ((c >> 24) & 255) as f32 / 255.0]
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_setDisplay(
    _: JNIEnv,
    _: JClass,
    db_min: jfloat,
    db_max: jfloat,
    spec: jfloat,
    fill: jint,
    line: jint,
    bg: jint,
    line_w: jfloat,
    palette: jint,
) {
    render::update(|p| {
        p.db_min = db_min;
        p.db_max = db_max;
        p.spectrum_fraction = spec;
        p.fill = argb(fill);
        p.line = argb(line);
        p.background = argb(bg);
        p.line_width = line_w;
        p.palette = palette;
    });
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_surfaceAttach(env: JNIEnv, _: JClass, surface: JObject) {
    // SAFETY: valid JNIEnv and Surface from the caller; the window reference is released by
    // the renderer
    let w = unsafe { super::ANativeWindow_fromSurface(env.get_raw() as *mut _, surface.as_raw() as *mut _) };
    if !w.is_null() {
        render::attach(w);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_surfaceResize(_: JNIEnv, _: JClass, _w: jint, _h: jint) {
    render::resize();
}

#[no_mangle]
pub extern "system" fn Java_app_littlesdr_engine_NativeBridge_surfaceDetach(_: JNIEnv, _: JClass) {
    render::detach();
}
