package dev.dexdrop;

import android.app.Activity;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;

/**
 * An invisible activity that shows the system's document picker
 * ({@code ACTION_OPEN_DOCUMENT}, several, any type) and hands the picked
 * {@code content://} URIs back to {@link AssetsBridge#pickDocuments}: the
 * documents stay where they are, read later through their provider
 * (no copy, unlike NSH's {@code pick_paths}). Declared by dexdrop-net's
 * Cargo.toml ([package.metadata.nsh.android] application): not exported,
 * translucent.
 */
public final class PickerActivity extends Activity {
    private static final int PICK = 1;
    private static final String TYPES = "types";
    private static BlockingQueue<List<Uri>> answer;

    /** Shows the picker and waits for the user; an empty list when cancelled. */
    static List<Uri> pick(Context context, String[] mimeTypes) {
        BlockingQueue<List<Uri>> queue = new ArrayBlockingQueue<>(1);
        synchronized (PickerActivity.class) {
            answer = queue;
        }
        Intent intent = new Intent(context, PickerActivity.class)
                .putExtra(TYPES, mimeTypes)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_NO_ANIMATION);
        context.startActivity(intent);
        try {
            return queue.take();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return new ArrayList<>();
        }
    }

    private static void deliver(List<Uri> uris) {
        BlockingQueue<List<Uri>> queue;
        synchronized (PickerActivity.class) {
            queue = answer;
            answer = null;
        }
        if (queue != null) {
            queue.offer(uris);
        }
    }

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        if (saved != null) {
            return; // Recreated while the picker shows: its result comes here.
        }
        String[] types = getIntent().getStringArrayExtra(TYPES);
        Intent open = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType("*/*")
                .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        if (types != null && types.length > 0 && !(types.length == 1 && types[0].isEmpty())) {
            open.putExtra(Intent.EXTRA_MIME_TYPES, types);
        }
        try {
            startActivityForResult(open, PICK);
        } catch (Exception e) {
            deliver(new ArrayList<>());
            finish();
        }
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        List<Uri> uris = new ArrayList<>();
        if (request == PICK && result == RESULT_OK && data != null) {
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    uris.add(clip.getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
        }
        deliver(uris);
        finish();
        overridePendingTransition(0, 0);
    }

    @Override
    protected void onDestroy() {
        // Gone without an answer (the system closed it): nothing picked.
        if (isFinishing()) {
            deliver(new ArrayList<>());
        }
        super.onDestroy();
    }
}
