package com.github.malba124.openmirroring.android.sender;

import android.content.Context;
import android.content.Intent;
import android.media.projection.MediaProjectionManager;
import android.os.Bundle;
import android.app.NativeActivity;
import android.util.Log;

import org.freedesktop.gstreamer.GStreamer;

public class MainActivity extends NativeActivity {
    private static final int REQUEST_CODE = 1;
    public static final String ACTION_RESULT =
            "com.github.malba124.openmirroring.android.sender.SCREEN_CAPTURE_RESULT";

    static {
        System.loadLibrary("gstreamer_android");
        System.loadLibrary("omandroidsender");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        try {
            GStreamer.init(this);
        } catch (Exception e) {
            Log.e("MAIN_ACTIVITY", "Failed to init GStreamer ${e}");
            finish();
            return;
        }
    }

    // Called from native code
    private void startScreenCapture() {
        MediaProjectionManager projectionManager =
                (MediaProjectionManager) getSystemService(Context.MEDIA_PROJECTION_SERVICE);
        startActivityForResult(projectionManager.createScreenCaptureIntent(), REQUEST_CODE);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQUEST_CODE && resultCode == RESULT_OK) {
            Intent intent = new Intent(this, ScreenCaptureService.class);
            intent.setAction(ACTION_RESULT);
            intent.putExtra("resultCode", resultCode);
            intent.putExtra("data", data);
            startForegroundService(intent);
        }
    }
}
