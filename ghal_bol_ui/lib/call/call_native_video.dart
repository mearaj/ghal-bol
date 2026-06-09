import "dart:async";
import "dart:convert";
import "dart:ui" as ui;

import "package:flutter/foundation.dart";
import "package:flutter/material.dart";

import "package:ghal_bol_ui/call/call_flow_log.dart";
import "package:ghal_bol_ui/call/call_video_texture_bridge.dart";
import "package:ghal_bol_ui/call/call_video_texture_pool.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";

/// Fallback I420→RGBA when poll path returns legacy `format: i420`.
Uint8List i420ToRgba(Uint8List i420, int width, int height) {
  final w = width;
  final h = height;
  final ySize = w * h;
  final uvW = w ~/ 2;
  final uvH = h ~/ 2;
  if (i420.length < ySize + 2 * uvW * uvH) {
    return Uint8List(w * h * 4);
  }
  final out = Uint8List(w * h * 4);
  var o = 0;
  for (var row = 0; row < h; row++) {
    for (var col = 0; col < w; col++) {
      final y = i420[row * w + col];
      final uvRow = row ~/ 2;
      final uvCol = col ~/ 2;
      final u = i420[ySize + uvRow * uvW + uvCol];
      final v = i420[ySize + uvW * uvH + uvRow * uvW + uvCol];
      final yf = y.toDouble();
      final uf = u - 128.0;
      final vf = v - 128.0;
      final r = (yf + 1.402 * vf).clamp(0.0, 255.0).round();
      final g = (yf - 0.344136 * uf - 0.714136 * vf).clamp(0.0, 255.0).round();
      final b = (yf + 1.772 * uf).clamp(0.0, 255.0).round();
      out[o++] = r;
      out[o++] = g;
      out[o++] = b;
      out[o++] = 255;
    }
  }
  return out;
}

Uint8List _prepareVideoRgba(Map<String, dynamic> args) {
  final bytes = base64Decode(args["b64"] as String);
  if (args["format"] == "rgba") return bytes;
  return i420ToRgba(bytes, args["width"] as int, args["height"] as int);
}

/// Native call video: GPU [Texture] on Android/Linux; poll fallback elsewhere.
class NativeCallVideoView extends StatefulWidget {
  const NativeCallVideoView({
    super.key,
    required this.callId,
    required this.track,
    this.active = true,
    this.mirror = false,
    this.quarterTurns = 0,
    this.autoRotateLandscape = false,
    this.objectFit = BoxFit.cover,
    this.onFrameReady,
  });

  final String callId;
  final String track;
  final bool active;
  final bool mirror;
  final int quarterTurns;
  final bool autoRotateLandscape;
  final BoxFit objectFit;
  final VoidCallback? onFrameReady;

  @override
  State<NativeCallVideoView> createState() => _NativeCallVideoViewState();
}

class _NativeCallVideoViewState extends State<NativeCallVideoView> {
  int? _textureId;
  int _textureW = 0;
  int _textureH = 0;

  // Poll fallback (macOS / Windows / texture registration failure).
  int _generation = 0;
  ui.Image? _image;
  int _frameW = 0;
  int _frameH = 0;
  int _decodeSeq = 0;
  int _loopGen = 0;
  bool _loggedFirstFrame = false;
  bool _usePollFallback = !CallVideoTextureBridge.supported;

  @override
  void initState() {
    super.initState();
    _restart();
  }

  @override
  void didUpdateWidget(covariant NativeCallVideoView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.callId != widget.callId ||
        oldWidget.active != widget.active ||
        oldWidget.track != widget.track) {
      _restart();
    }
  }

  void _restart() {
    _loopGen++;
    _decodeSeq++;
    _loggedFirstFrame = false;
    _textureId = null;
    _image?.dispose();
    _image = null;
    _textureId = null;
    _textureW = 0;
    _textureH = 0;
    if (!widget.active || widget.callId.isEmpty) return;
    if (_usePollFallback) {
      unawaited(_fetchLoop(_loopGen));
    } else {
      unawaited(_textureSetupLoop(_loopGen));
    }
  }

  @override
  void dispose() {
    _loopGen++;
    _textureId = null;
    _image?.dispose();
    super.dispose();
  }

  void _applyTexture(int id, int w, int h) {
    if (!_loggedFirstFrame) {
      _loggedFirstFrame = true;
      CallFlowLog.step("video_texture_rx", {
        "track": widget.track,
        "w": w.toString(),
        "h": h.toString(),
      });
      widget.onFrameReady?.call();
    }
    setState(() {
      _textureId = id;
      _textureW = w;
      _textureH = h;
    });
  }

  Future<void> _textureSetupLoop(int gen) async {
    final cached = CallVideoTexturePool.peek(widget.callId, widget.track);
    if (cached != null && mounted && gen == _loopGen) {
      CallVideoTexturePool.retain(
        widget.callId,
        widget.track,
        cached.id,
        cached.w,
        cached.h,
      );
      _applyTexture(cached.id, cached.w, cached.h);
      return;
    }

    var attempts = 0;
    var backoffMs = 32;
    while (mounted && widget.active && gen == _loopGen && widget.callId.isNotEmpty) {
      if (++attempts > 90) {
        CallFlowLog.issue(
          "video_texture_setup_timeout",
          check: "shm ready + embedder register",
          detail: "track=${widget.track}",
        );
        _usePollFallback = true;
        unawaited(_fetchLoop(gen));
        return;
      }
      final r = await GhalBolP2p.callVideoTexture(
        callId: widget.callId,
        track: widget.track,
      );
      if (!mounted || gen != _loopGen) return;
      if (r["ok"] == true && r["ready"] == true) {
        final path = r["shm_path"]?.toString();
        final w = r["width"];
        final h = r["height"];
        if (path != null &&
            path.isNotEmpty &&
            w is num &&
            h is num &&
            w > 0 &&
            h > 0) {
          final id = await CallVideoTextureBridge.register(
            shmPath: path,
            width: w.toInt(),
            height: h.toInt(),
          );
          if (!mounted || gen != _loopGen) return;
          if (id != null) {
            CallVideoTexturePool.retain(
              widget.callId,
              widget.track,
              id,
              w.toInt(),
              h.toInt(),
            );
            _applyTexture(id, w.toInt(), h.toInt());
            return;
          }
        }
      }
      await Future<void>.delayed(Duration(milliseconds: backoffMs));
      backoffMs = (backoffMs * 1.4).round().clamp(32, 400);
    }
  }

  Future<void> _fetchLoop(int gen) async {
    while (mounted && widget.active && widget.callId.isNotEmpty && gen == _loopGen) {
      final hadFrame = await _fetchOnce();
      if (!mounted || gen != _loopGen) break;
      if (!hadFrame) {
        await Future<void>.delayed(const Duration(milliseconds: 16));
      }
    }
  }

  Future<bool> _fetchOnce() async {
    if (!mounted || !widget.active || widget.callId.isEmpty) return false;
    final r = await GhalBolP2p.callVideoFrame(
      callId: widget.callId,
      sinceGeneration: _generation,
      track: widget.track,
    );
    if (!mounted || !widget.active) return false;
    if (r["ok"] != true || r["has_frame"] != true) return false;

    final gen = r["generation"];
    if (gen is! num) return false;
    final w = r["width"];
    final h = r["height"];
    if (w is! num || h is! num || w <= 0 || h <= 0) return false;
    final b64 = r["data_base64"]?.toString();
    if (b64 == null || b64.isEmpty) return false;

    final decodeSeq = ++_decodeSeq;
    unawaited(_decodeAndShow(
      decodeSeq: decodeSeq,
      generation: gen.toInt(),
      width: w.toInt(),
      height: h.toInt(),
      format: r["format"]?.toString() ?? "rgba",
      dataBase64: b64,
    ));
    return true;
  }

  Future<void> _decodeAndShow({
    required int decodeSeq,
    required int generation,
    required int width,
    required int height,
    required String format,
    required String dataBase64,
  }) async {
    if (!mounted || decodeSeq != _decodeSeq) return;
    try {
      final rgba = await compute(_prepareVideoRgba, {
        "b64": dataBase64,
        "format": format,
        "width": width,
        "height": height,
      });
      if (!mounted || decodeSeq != _decodeSeq) return;
      final buffer = await ui.ImmutableBuffer.fromUint8List(rgba);
      final descriptor = ui.ImageDescriptor.raw(
        buffer,
        width: width,
        height: height,
        pixelFormat: ui.PixelFormat.rgba8888,
      );
      final codec = await descriptor.instantiateCodec();
      final frame = await codec.getNextFrame();
      descriptor.dispose();
      codec.dispose();
      final img = frame.image;
      if (!mounted || decodeSeq != _decodeSeq) {
        img.dispose();
        return;
      }
      if (!_loggedFirstFrame) {
        _loggedFirstFrame = true;
        CallFlowLog.step("video_frame_rx", {
          "track": widget.track,
          "w": width.toString(),
          "h": height.toString(),
        });
        widget.onFrameReady?.call();
      }
      setState(() {
        _generation = generation;
        _frameW = width;
        _frameH = height;
        _image?.dispose();
        _image = img;
      });
    } catch (_) {}
  }

  Widget _layoutFrame(Widget frame, double fw, double fh) {
    var turns = widget.quarterTurns;
    final rotateLandscape =
        widget.autoRotateLandscape && fw > fh;
    if (rotateLandscape) {
      turns = 3;
    }
    if (widget.mirror) {
      frame = Transform(
        alignment: Alignment.center,
        transform: Matrix4.rotationY(3.141592653589793),
        child: frame,
      );
    }
    if (turns != 0) {
      frame = RotatedBox(quarterTurns: turns, child: frame);
    }
    final swap = turns.isOdd;
    final layoutW = swap ? fh : fw;
    final layoutH = swap ? fw : fh;
    return SizedBox.expand(
      child: FittedBox(
        fit: widget.objectFit,
        child: SizedBox(
          width: layoutW,
          height: layoutH,
          child: frame,
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final texId = _textureId;
    if (texId != null) {
      final fw = (_textureW > 0 ? _textureW : 640).toDouble();
      final fh = (_textureH > 0 ? _textureH : 480).toDouble();
      return _layoutFrame(Texture(textureId: texId), fw, fh);
    }

    final img = _image;
    if (img == null) {
      return const ColoredBox(color: Color(0xFF1A1D26));
    }
    final fw = _frameW > 0 ? _frameW.toDouble() : img.width.toDouble();
    final fh = _frameH > 0 ? _frameH.toDouble() : img.height.toDouble();
    return _layoutFrame(
      RawImage(image: img, width: fw, height: fh, fit: BoxFit.fill),
      fw,
      fh,
    );
  }
}
