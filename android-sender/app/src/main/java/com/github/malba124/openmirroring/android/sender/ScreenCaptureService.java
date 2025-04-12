package com.github.malba124.openmirroring.android.sender;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.graphics.PixelFormat;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.media.Image;
import android.media.ImageReader;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.util.DisplayMetrics;
import android.util.Log;
import android.view.WindowManager;

import androidx.annotation.Nullable;
import androidx.core.app.NotificationCompat;

import java.nio.ByteBuffer;

public class ScreenCaptureService extends Service {
    private MediaProjectionManager mediaProjectionManager;
    private MediaProjection mediaProjection;
    private ImageReader imageReader;
    private VirtualDisplay virtualDisplay;
    private ProjectionCallback projectionCallback;
    private Handler handler;

    public ScreenCaptureService() {
    }

    @Override
    public void onCreate() {
        super.onCreate();
        handler = new Handler(Looper.getMainLooper());
        projectionCallback = new ProjectionCallback();
        mediaProjectionManager =
                (MediaProjectionManager) getSystemService(MEDIA_PROJECTION_SERVICE);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent.getAction().equals(MainActivity.ACTION_RESULT)) {
            startForegroundService();

            int resultCode = intent.getIntExtra("resultCode", Activity.RESULT_CANCELED);
            Intent data = intent.getParcelableExtra("data");
            startScreenCapture(resultCode, data);
        }
        return START_STICKY;
    }

    private void startForegroundService() {
        Log.d("SCREEN_CAPTURE", "starting foreground service");

        String channelId = "ScreenCaptureChannel";

        // For API >=26, we're at least that, always
        NotificationChannel channel = new NotificationChannel(
                channelId,
                "Screen Capture Service",
                NotificationManager.IMPORTANCE_LOW
        );
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(channel);

        Notification notification = new NotificationCompat.Builder(this, channelId)
                .setContentTitle("Screen Capture")
                .setContentText("Capturing screen...")
                .build();

        startForeground(1, notification);
    }

    private void startScreenCapture(int resultCode, Intent data) {
        Log.d("SCREEN_CAPTURE", "starting screen capture");

        mediaProjection = mediaProjectionManager.getMediaProjection(resultCode, data);

        mediaProjection.registerCallback(projectionCallback, handler);

        setupVirtualDisplay();
    }

    private void setupVirtualDisplay() {
        DisplayMetrics metrics = new DisplayMetrics();
        WindowManager windowManager = (WindowManager) getSystemService(Context.WINDOW_SERVICE);
        windowManager.getDefaultDisplay().getMetrics(metrics);

        int width = metrics.widthPixels;
        int height = metrics.heightPixels;
        int density = metrics.densityDpi;

        imageReader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 2);
        imageReader.setOnImageAvailableListener(reader -> {
            try (Image image = reader.acquireLatestImage()) {
                if (image == null) {
                    return;
                }

                processFrame(image);
            }
        }, new Handler(Looper.getMainLooper()));

        virtualDisplay = mediaProjection.createVirtualDisplay(
                "ScreenCapture",
                width,
                height,
                density,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                imageReader.getSurface(),
                null,
                null
        );
    }

    private void processFrame(Image image) {
        Image.Plane[] planes = image.getPlanes();
        ByteBuffer buffer = planes[0].getBuffer();
        int pixelStride = planes[0].getPixelStride();
        int rowStride = planes[0].getRowStride();
        int width = image.getWidth();
        int height = image.getHeight();
        nativeProcessFrame(buffer, width, height, pixelStride, rowStride);
    }

    public void stopCapture() {
        if (virtualDisplay != null) {
            virtualDisplay.release();
            virtualDisplay = null;
        }
        if (imageReader != null) {
            imageReader.close();
            imageReader = null;
        }
        if (mediaProjection != null) {
            mediaProjection.stop();
            mediaProjection = null;
        }
        stopForeground(true);
        stopSelf();
    }

    public class ProjectionCallback extends MediaProjection.Callback {
        @Override
        public void onStop() {
            stopCapture();
        }
    }

    @Override
    public void onDestroy() {
        stopCapture();
        super.onDestroy();
    }

    @Nullable
    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private native void nativeProcessFrame(
            ByteBuffer buffer,
            int width,
            int height,
            int pixelStride,
            int rowStride
    );
}