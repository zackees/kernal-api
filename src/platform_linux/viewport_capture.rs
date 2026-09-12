//! UI-thread WebKitGTK viewport capture. Never returns native images or PNG
//! vectors through the guest boundary; the encoder writes into the shared hub.

use crate::operations::{NativeBlobEncoder, OpaqueToken, OperationHub};
use gio::prelude::CancellableExt;
use gtk::prelude::WidgetExt;
use std::sync::Arc;
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
use wry::WebViewExtUnix;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureError {
    InvalidDimensions,
    PixelLimit,
    BlobLimit,
    Cancelled,
    NativeFailure,
    EncodingFailure,
}

fn check_pixels(width: i32, height: i32, scale: i32, maximum: u64) -> Result<(), CaptureError> {
    if width <= 0 || height <= 0 || scale <= 0 || maximum == 0 {
        return Err(CaptureError::InvalidDimensions);
    }
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(scale as u64))
        .and_then(|pixels| pixels.checked_mul(scale as u64))
        .ok_or(CaptureError::PixelLimit)?;
    if pixels > maximum {
        return Err(CaptureError::PixelLimit);
    }
    Ok(())
}

// The native operation service will retain/cancel the returned cancellable on
// its UI thread. This adapter is not yet exposed as a generated operation.
#[allow(dead_code)]
pub(crate) fn capture(
    view: &wry::WebView,
    hub: Arc<OperationHub>,
    store: u64,
    maximum_pixels: u64,
    maximum_encoded_bytes: usize,
    completed: impl FnOnce(Result<OpaqueToken, CaptureError>) + 'static,
) -> Result<gio::Cancellable, CaptureError> {
    let native = view.webview();
    check_pixels(
        native.allocated_width(),
        native.allocated_height(),
        native.scale_factor(),
        maximum_pixels,
    )?;
    let encoder = NativeBlobEncoder::new(hub, store, maximum_encoded_bytes)
        .map_err(|_| CaptureError::BlobLimit)?;
    let cancellation = gio::Cancellable::new();
    let callback_cancellation = cancellation.clone();
    native.snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        Some(&cancellation),
        move |result| {
            let result = if callback_cancellation.is_cancelled() {
                Err(CaptureError::Cancelled)
            } else {
                result
                    .map_err(|_| CaptureError::NativeFailure)
                    .and_then(|surface| {
                        encode_surface(surface, encoder, maximum_pixels, &callback_cancellation)
                    })
            };
            completed(result);
        },
    );
    Ok(cancellation)
}

fn encode_surface(
    surface: cairo::Surface,
    mut encoder: NativeBlobEncoder,
    maximum_pixels: u64,
    cancellation: &gio::Cancellable,
) -> Result<OpaqueToken, CaptureError> {
    if cancellation.is_cancelled() {
        return Err(CaptureError::Cancelled);
    }
    let image = cairo::ImageSurface::try_from(surface).map_err(|_| CaptureError::NativeFailure)?;
    check_pixels(image.width(), image.height(), 1, maximum_pixels)?;
    if image.write_to_png(&mut encoder).is_err() {
        return Err(match encoder.failure() {
            Some(crate::operations::HubError::Quota) => CaptureError::BlobLimit,
            Some(crate::operations::HubError::Closed | crate::operations::HubError::Invalid) => {
                CaptureError::Cancelled
            }
            _ => CaptureError::EncodingFailure,
        });
    }
    if cancellation.is_cancelled() {
        return Err(CaptureError::Cancelled);
    }
    encoder.finish().map_err(|_| CaptureError::BlobLimit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_pixel_limit_checks_scale_and_overflow() {
        assert_eq!(check_pixels(10, 20, 2, 800), Ok(()));
        assert_eq!(check_pixels(10, 20, 2, 799), Err(CaptureError::PixelLimit));
        assert_eq!(
            check_pixels(i32::MAX, i32::MAX, i32::MAX, u64::MAX),
            Err(CaptureError::PixelLimit)
        );
        assert_eq!(
            check_pixels(0, 20, 1, 800),
            Err(CaptureError::InvalidDimensions)
        );
    }

    #[test]
    fn cairo_png_is_decodable_and_uses_only_the_shared_blob_sink() {
        let hub = OperationHub::with_blob_limits(
            4,
            2,
            crate::operations::BlobLimits::new(256, 65_536, 65_536).unwrap(),
        )
        .unwrap();
        let image = cairo::ImageSurface::create(cairo::Format::ARgb32, 8, 8).unwrap();
        let context = cairo::Context::new(&image).unwrap();
        context.set_source_rgb(1.0, 0.0, 0.0);
        context.paint().unwrap();
        drop(context);
        let encoder = NativeBlobEncoder::new(Arc::clone(&hub), 1, 65_536).unwrap();
        let blob = encode_surface(
            image.as_ref().clone(),
            encoder,
            64,
            &gio::Cancellable::new(),
        )
        .unwrap();
        let mut encoded = Vec::new();
        loop {
            let chunk = hub.blob_read(1, blob, 256).unwrap();
            if chunk.is_empty() {
                break;
            }
            encoded.extend(chunk);
        }
        let decoded =
            cairo::ImageSurface::create_from_png(&mut std::io::Cursor::new(encoded)).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
        hub.close_resource(blob).unwrap();
        assert_eq!(hub.snapshot().retained_transfer_capacity, 0);
        assert_eq!(hub.snapshot().live_blobs, 0);
    }

    #[test]
    fn actual_surface_limit_and_cancel_reclaim_reserved_blob() {
        for cancelled in [false, true] {
            let hub = OperationHub::with_blob_limits(
                4,
                2,
                crate::operations::BlobLimits::new(256, 65_536, 65_536).unwrap(),
            )
            .unwrap();
            let image = cairo::ImageSurface::create(cairo::Format::ARgb32, 8, 8).unwrap();
            let encoder = NativeBlobEncoder::new(Arc::clone(&hub), 1, 65_536).unwrap();
            let cancellation = gio::Cancellable::new();
            if cancelled {
                cancellation.cancel();
            }
            assert_eq!(
                encode_surface(image.as_ref().clone(), encoder, 63, &cancellation),
                Err(if cancelled {
                    CaptureError::Cancelled
                } else {
                    CaptureError::PixelLimit
                })
            );
            assert_eq!(hub.snapshot().live_blobs, 0);
            assert_eq!(hub.snapshot().retained_transfer_capacity, 0);
        }
    }
}
