const CALL_MEDIA_MAX_FRAME: usize = 64 * 1024;

fn derive_call_media_keys_for_peer(
    session: &SessionState,
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<crate::call_media_key::CallMediaKeys, String> {
    let peer_wire =
        crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire)?;
    let peer_transport_pk = session
        .peer_transport_pk(&peer_wire)
        .ok_or_else(|| "call media: transport kem not ready for peer".to_string())?;
    crate::call_media_key::derive_call_media_keys_from_transport(
        session.dm_local_transport_sk(),
        &peer_transport_pk,
        &session.identity.identity_wire(),
        &peer_wire,
        call_id,
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CallStreamHeader {
    call_id: String,
}

/// Length-prefixed (u32 LE) body write for the media substream.
async fn write_media_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    body: &[u8],
) -> Result<(), String> {
    if body.len() > CALL_MEDIA_MAX_FRAME {
        return Err("call media frame too large".to_string());
    }
    writer
        .write_all(&(body.len() as u32).to_le_bytes())
        .await
        .map_err(|e| format!("write len: {e}"))?;
    writer
        .write_all(body)
        .await
        .map_err(|e| format!("write body: {e}"))?;
    writer.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

/// Length-prefixed body read for the media substream.
async fn read_media_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("read len: {e}"))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > CALL_MEDIA_MAX_FRAME {
        return Err("call media frame too large".to_string());
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("read body: {e}"))?;
    Ok(body)
}

/// Start native voice media for `call_id`: derive identity media keys, open the
/// audio device, spawn the engine session, and open our TX media substream. The
/// peer's audio arrives on a separate inbound substream (see
/// [`handle_inbound_call_stream`]). Idempotent per `call_id`.
async fn start_call_media(
    session: Arc<SessionState>,
    mut control: stream::Control,
    call_id: String,
    peer_public_key_hex: String,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    let pk = peer_public_key_hex.trim().to_string();
    if !crate::public_key_util::is_valid_contact_identity(&pk) {
        return Err("call media: invalid peer identity".to_string());
    }
    if session.call_media_active(&call_id) {
        crate::p2p::call_active::on_voice_start(&call_id, &pk);
        emit_call_media(&events_tx, &call_id, &pk, "voice_started", None);
        return Ok(());
    }
    let peer = session
        .resolve_send_peer(&pk)
        .ok_or_else(|| "call media: unknown contact".to_string())?;
    let keys = derive_call_media_keys_for_peer(session.as_ref(), &pk, &call_id)?;
    let local_is_a =
        crate::call_media::local_is_a(&session.identity.identity_wire(), &pk);
    let engine = crate::call_media::MediaEngine::new_opus(&keys.frame_key, local_is_a)?;

    #[cfg(target_os = "android")]
    if let Err(e) = crate::call_media::ensure_voice_audio_mode() {
        native_log::warn("call_media", format!("android audio mode: {e}"));
    }

    let mut backend = crate::call_media::default_audio_backend();
    let audio = backend
        .start()
        .map_err(|e| format!("call media: audio start: {e}"))?;
    let controls = crate::call_media::MediaControls::new();

    let (wire_out_tx, mut wire_out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (wire_in_tx, wire_in_rx) = mpsc::channel::<Vec<u8>>(256);

    // Register before opening the TX stream so a racing inbound RX stream can attach.
    session.call_media_register(call_id.clone(), peer, controls.clone(), wire_in_tx);

    native_log::info(
        "call_media",
        format!("start call_id={call_id} peer={peer} local_is_a={local_is_a}"),
    );

    // Engine session task (owns engine + audio backend for the call lifetime).
    let session_ctl = controls.clone();
    tokio::spawn(async move {
        crate::call_media::run_media_session(engine, audio, wire_out_tx, wire_in_rx, session_ctl)
            .await;
        backend.stop();
    });

    // TX task: open our outbound media substream, send the header, then pump
    // sealed frames from the engine until the call stops or the stream breaks.
    let tx_ctl = controls.clone();
    let tx_call_id = call_id.clone();
    tokio::spawn(async move {
        let mut stream = match control
            .open_stream(peer, StreamProtocol::new(CALL_STREAM_PROTOCOL))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                native_log::warn(
                    "call_media",
                    format!("open media stream to {peer} failed: {e}"),
                );
                tx_ctl.request_stop();
                return;
            }
        };
        let header = serde_json::to_vec(&CallStreamHeader {
            call_id: tx_call_id.clone(),
        })
        .unwrap_or_default();
        if let Err(e) = write_media_frame(&mut stream, &header).await {
            native_log::warn("call_media", format!("media header write failed: {e}"));
            tx_ctl.request_stop();
            return;
        }
        loop {
            if tx_ctl.is_stopped() {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(250), wire_out_rx.recv()).await {
                Ok(Some(bytes)) => {
                    if let Err(e) = write_media_frame(&mut stream, &bytes).await {
                        native_log::warn("call_media", format!("media write ended: {e}"));
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {} // idle tick — re-check stop flag
            }
        }
        tx_ctl.request_stop();
        native_log::info(
            "call_media",
            format!("tx stream closed call_id={tx_call_id}"),
        );
    });

    // Lightweight stats logger so device tests show audio flowing without FFI.
    let stats_session = Arc::clone(&session);
    let stats_call_id = call_id.clone();
    let stats_ctl = controls;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            if stats_ctl.is_stopped() || !stats_session.call_media_active(&stats_call_id) {
                break;
            }
            native_log::info(
                "call_media",
                format!(
                    "call_id={stats_call_id} sent={} recv={}",
                    stats_ctl.sent(),
                    stats_ctl.received()
                ),
            );
        }
    });

    crate::p2p::call_active::on_voice_start(&call_id, &pk);
    emit_call_media(&events_tx, &call_id, &pk, "voice_started", None);
    Ok(())
}

/// Handle an inbound `/ghal-bol/call/1.0.0` substream: read the header to learn
/// the `call_id`, then forward sealed frames into that call's engine (RX path).
async fn handle_inbound_call_stream(
    peer: PeerId,
    mut stream: libp2p::Stream,
    session: Arc<SessionState>,
) {
    let header = match read_media_frame(&mut stream).await {
        Ok(h) => h,
        Err(e) => {
            native_log::warn("call_media", format!("inbound media header read: {e}"));
            return;
        }
    };
    let call_id = match serde_json::from_slice::<CallStreamHeader>(&header) {
        Ok(h) => h.call_id,
        Err(e) => {
            native_log::warn("call_media", format!("inbound media header parse: {e}"));
            return;
        }
    };

    // The peer's media stream can arrive a touch before our local CallMediaStart;
    // wait briefly for the engine to register.
    let mut wire_in = None;
    for _ in 0..75 {
        if let Some(tx) = session.call_media_wire_in_for_peer(&call_id, peer) {
            wire_in = Some(tx);
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let Some(wire_in) = wire_in else {
        native_log::warn(
            "call_media",
            format!("inbound media for unknown call_id={call_id} from {peer} — dropped"),
        );
        return;
    };
    native_log::info(
        "call_media",
        format!("rx stream attached call_id={call_id} peer={peer}"),
    );
    loop {
        match read_media_frame(&mut stream).await {
            Ok(bytes) => {
                if wire_in.send(bytes).await.is_err() {
                    break; // engine gone
                }
            }
            Err(e) => {
                if !stream_read_is_terminal(&e) {
                    continue;
                }
                break;
            }
        }
    }
    native_log::info("call_media", format!("rx stream closed call_id={call_id}"));
}

/// Start native **video** for `call_id`: derive a distinct video media key, build the
/// H.264 engine, start camera capture (receive-only if no camera), spawn the engine
/// session, and open our TX video substream. Decoded frames land in the per-call
/// frame registry for the FFI/daemon render pull. Idempotent per `call_id`.
async fn start_call_video(
    session: Arc<SessionState>,
    mut control: stream::Control,
    call_id: String,
    peer_public_key_hex: String,
    camera_enabled: bool,
) -> Result<(), String> {
    use crate::call_video::{
        DEFAULT_REASSEMBLY_PENDING, DEFAULT_VIDEO_JITTER_MAX, RawVideoFrame, VideoControls,
        VideoEngine, VideoStreams, run_video_session,
    };

    let pk = peer_public_key_hex.trim().to_string();
    if !crate::public_key_util::is_valid_contact_identity(&pk) {
        return Err("call video: invalid peer identity".to_string());
    }
    if session.call_video_active(&call_id) {
        if camera_enabled {
            session.call_video_set_camera_off(&call_id, false);
            crate::p2p::call_active::set_camera_on(&call_id, true);
        }
        crate::p2p::call_active::on_video_start(&call_id, &pk, camera_enabled);
        return Ok(());
    }
    crate::call_video::track_call_shm(&call_id);
    let peer = session
        .resolve_send_peer(&pk)
        .ok_or_else(|| "call video: unknown contact".to_string())?;

    // Distinct key from the audio stream (different HKDF salt via a `:video` suffix),
    // so audio and video never share a (key, nonce) space. Both peers derive the same.
    let video_key_id = format!("{call_id}:video");
    let keys = derive_call_media_keys_for_peer(session.as_ref(), &pk, &video_key_id)?;
    let local_is_a =
        crate::call_media::local_is_a(&session.identity.identity_wire(), &pk);
    // 16 KiB chunks: well under the 64 KiB substream frame cap, fewer writes per frame.
    let engine = VideoEngine::with_params(
        &keys.frame_key,
        local_is_a,
        Box::new(crate::call_video::H264Encoder::new()?),
        Box::new(crate::call_video::H264Decoder::new()?),
        16 * 1024,
        DEFAULT_REASSEMBLY_PENDING,
        DEFAULT_VIDEO_JITTER_MAX,
    );

    let controls = VideoControls::new();
    let (wire_out_tx, mut wire_out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (wire_in_tx, wire_in_rx) = mpsc::channel::<Vec<u8>>(256);

    // Register before opening TX so a racing inbound RX stream can attach.
    session.call_video_register(call_id.clone(), peer, controls.clone(), wire_in_tx);
    if camera_enabled {
        controls.set_camera_off(false);
    }

    native_log::info(
        "call_video",
        format!(
            "start call_id={call_id} peer={peer} local_is_a={local_is_a} camera_enabled={camera_enabled}",
        ),
    );

    // Camera capture (native). Receive-only if no camera is available — the call
    // still shows the peer's video; we just send nothing.
    let capture_rx = match crate::call_video::spawn_camera_capture(controls.clone()) {
        Ok(rx) => rx,
        Err(e) => {
            native_log::warn(
                "call_video",
                format!("camera unavailable ({e}) — receive-only call_id={call_id}"),
            );
            let (keep_tx, rx) = mpsc::channel::<RawVideoFrame>(1);
            let ctl = controls.clone();
            tokio::spawn(async move {
                // Hold the capture sender open so the session stays alive (RX only).
                while !ctl.is_stopped() {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                drop(keep_tx);
            });
            rx
        }
    };

    // Render sink: engine → global frame registry for the FFI render pull.
    let (render_tx, mut render_rx) = mpsc::channel::<RawVideoFrame>(8);
    let render_call_id = call_id.clone();
    tokio::spawn(async move {
        while let Some(frame) = render_rx.recv().await {
            crate::call_video::publish_decoded_frame(&render_call_id, frame);
        }
    });

    let streams = VideoStreams {
        capture_rx,
        render_tx,
    };
    let session_ctl = controls.clone();
    let session_call_id = call_id.clone();
    tokio::spawn(async move {
        run_video_session(
            engine,
            streams,
            session_call_id,
            wire_out_tx,
            wire_in_rx,
            session_ctl,
        )
        .await;
    });

    // TX task: open our outbound video substream, send the header, pump sealed chunks.
    let tx_ctl = controls.clone();
    let tx_call_id = call_id.clone();
    tokio::spawn(async move {
        let mut stream = match control
            .open_stream(peer, StreamProtocol::new(CALL_VIDEO_STREAM_PROTOCOL))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                native_log::warn(
                    "call_video",
                    format!("open video stream to {peer} failed: {e}"),
                );
                tx_ctl.request_stop();
                return;
            }
        };
        let header = serde_json::to_vec(&CallStreamHeader {
            call_id: tx_call_id.clone(),
        })
        .unwrap_or_default();
        if let Err(e) = write_media_frame(&mut stream, &header).await {
            native_log::warn("call_video", format!("video header write failed: {e}"));
            tx_ctl.request_stop();
            return;
        }
        loop {
            if tx_ctl.is_stopped() {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(250), wire_out_rx.recv()).await {
                Ok(Some(bytes)) => {
                    if let Err(e) = write_media_frame(&mut stream, &bytes).await {
                        native_log::warn("call_video", format!("video write ended: {e}"));
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {}
            }
        }
        tx_ctl.request_stop();
        native_log::info(
            "call_video",
            format!("tx stream closed call_id={tx_call_id}"),
        );
    });

    crate::p2p::call_active::on_video_start(&call_id, &pk, camera_enabled);
    Ok(())
}

/// Handle an inbound `/ghal-bol/call-video/1.0.0` substream: read the header for the
/// `call_id`, then forward sealed chunks into that call's video engine (RX path).
async fn handle_inbound_call_video_stream(
    peer: PeerId,
    mut stream: libp2p::Stream,
    session: Arc<SessionState>,
) {
    let header = match read_media_frame(&mut stream).await {
        Ok(h) => h,
        Err(e) => {
            native_log::warn("call_video", format!("inbound video header read: {e}"));
            return;
        }
    };
    let call_id = match serde_json::from_slice::<CallStreamHeader>(&header) {
        Ok(h) => h.call_id,
        Err(e) => {
            native_log::warn("call_video", format!("inbound video header parse: {e}"));
            return;
        }
    };
    let mut wire_in = None;
    for _ in 0..75 {
        if let Some(tx) = session.call_video_wire_in_for_peer(&call_id, peer) {
            wire_in = Some(tx);
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let Some(wire_in) = wire_in else {
        native_log::warn(
            "call_video",
            format!("inbound video for unknown call_id={call_id} from {peer} — dropped"),
        );
        return;
    };
    native_log::info(
        "call_video",
        format!("rx video stream attached call_id={call_id} peer={peer}"),
    );
    loop {
        match read_media_frame(&mut stream).await {
            Ok(bytes) => {
                if wire_in.send(bytes).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                if !stream_read_is_terminal(&e) {
                    continue;
                }
                break;
            }
        }
    }
    native_log::info(
        "call_video",
        format!("rx video stream closed call_id={call_id}"),
    );
}

