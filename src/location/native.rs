//! Android native GPS via `android.location.LocationManager` (JNI).
//!
//! The [`AndroidGpsSource`] polls `getLastKnownLocation` for the best available
//! provider on a background thread and pushes each fix through the location
//! `watch` channel.  The JavaVM and Activity are obtained from
//! [`ndk_context`], which `android-activity`/`winit` install globally — so the
//! source needs nothing wired through from the render core.
//!
//! Every JNI step is fallible and handled defensively: a missing permission,
//! an unavailable provider, or an un-initialised context yields `None` for that
//! poll rather than panicking, so in the worst case the source behaves like the
//! old stub (no fixes) instead of crashing the app.
//!
//! Runtime permission: `ACCESS_FINE_LOCATION` is declared in the manifest.  On
//! Android 6+ it must also be granted at runtime; until it is,
//! `getLastKnownLocation` throws `SecurityException` and this source simply
//! reports no fix.  Requesting the runtime grant is a separate concern owned by
//! the Android activity layer.

use tokio::sync::watch;

use super::source::{LocationFrame, LocationSource};

/// How often to poll for a new fix.
#[cfg(target_os = "android")]
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

pub struct AndroidGpsSource;

impl LocationSource for AndroidGpsSource {
    fn name(&self) -> &str {
        "native"
    }

    #[cfg(target_os = "android")]
    fn start(
        self,
        rt: &tokio::runtime::Handle,
        tx: watch::Sender<Option<LocationFrame>>,
    ) -> tokio::task::JoinHandle<()> {
        rt.spawn(async move {
            tracing::info!("location: native Android GPS started");
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            loop {
                ticker.tick().await;
                // JNI calls block; keep them off the async worker threads.
                let fix = tokio::task::spawn_blocking(poll_last_known_fix)
                    .await
                    .ok()
                    .flatten();
                if let Some(frame) = fix {
                    // A send error means every receiver was dropped → shut down.
                    if tx.send(Some(frame)).is_err() {
                        break;
                    }
                }
            }
        })
    }

    #[cfg(not(target_os = "android"))]
    fn start(
        self,
        rt: &tokio::runtime::Handle,
        tx: watch::Sender<Option<LocationFrame>>,
    ) -> tokio::task::JoinHandle<()> {
        rt.spawn(async move {
            tracing::warn!(
                "location: native GPS is Android-only — use 'mock' or 'network' on desktop"
            );
            let _ = tx.send(None);
            // Park so LocationManager's abort() works uniformly across sources.
            std::future::pending::<()>().await;
        })
    }
}

/// Query the Android `LocationManager` for the most recent fix across the
/// preferred providers.  Returns `None` on any failure (no permission, no
/// provider, no cached fix, or no Android context).
///
/// Runs on a blocking thread; performs only JNI work and returns a plain
/// [`LocationFrame`] so the result is `Send`.
#[cfg(target_os = "android")]
fn poll_last_known_fix() -> Option<LocationFrame> {
    // A panic here (e.g. a not-yet-initialised context) must never reach the
    // app, so isolate the JNI work behind catch_unwind.
    std::panic::catch_unwind(jni_poll).ok().flatten()
}

#[cfg(target_os = "android")]
fn jni_poll() -> Option<LocationFrame> {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    // Provider order: most accurate first.  "fused" exists on API 31+.
    const PROVIDERS: &[&str] = &["fused", "gps", "network", "passive"];

    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

    // LocationManager lm = (LocationManager) activity.getSystemService("location");
    let service_name = env.new_string("location").ok()?;
    let location_manager = env
        .call_method(
            &activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service_name)],
        )
        .ok()?
        .l()
        .ok()?;

    // Take the freshest fix across providers (by Location.getTime()).
    let mut best: Option<LocationFrame> = None;
    for provider in PROVIDERS {
        let Some(frame) = last_known_for(&mut env, &location_manager, provider) else {
            // Clear any pending exception (e.g. unknown provider) before retrying.
            let _ = env.exception_clear();
            continue;
        };
        if best.as_ref().map(|b| frame.ts > b.ts).unwrap_or(true) {
            best = Some(frame);
        }
    }
    best
}

/// Read `LocationManager.getLastKnownLocation(provider)` and unpack it into a
/// [`LocationFrame`], or `None` if the provider has no cached fix.
#[cfg(target_os = "android")]
fn last_known_for(
    env: &mut jni::JNIEnv,
    location_manager: &jni::objects::JObject,
    provider: &str,
) -> Option<LocationFrame> {
    use jni::objects::JValue;

    let provider_str = env.new_string(provider).ok()?;
    let location = env
        .call_method(
            location_manager,
            "getLastKnownLocation",
            "(Ljava/lang/String;)Landroid/location/Location;",
            &[JValue::Object(&provider_str)],
        )
        .ok()?
        .l()
        .ok()?;
    if location.is_null() {
        return None;
    }

    let d = |env: &mut jni::JNIEnv, method: &str| -> Option<f64> {
        env.call_method(&location, method, "()D", &[])
            .ok()?
            .d()
            .ok()
    };
    let f = |env: &mut jni::JNIEnv, method: &str| -> Option<f64> {
        env.call_method(&location, method, "()F", &[])
            .ok()?
            .f()
            .ok()
            .map(|v| v as f64)
    };

    let lat = d(env, "getLatitude")?;
    let lon = d(env, "getLongitude")?;
    let alt = d(env, "getAltitude").unwrap_or(0.0);
    let acc = f(env, "getAccuracy").unwrap_or(f64::NAN);
    let spd = f(env, "getSpeed").unwrap_or(0.0);
    let hdg = f(env, "getBearing").unwrap_or(f64::NAN);
    let ts = env
        .call_method(&location, "getTime", "()J", &[])
        .ok()?
        .j()
        .ok()
        .map(|v| v as u64)
        .unwrap_or(0);

    Some(LocationFrame {
        lat,
        lon,
        alt,
        hdg,
        spd,
        acc,
        ts,
    })
}
