#include "call_video_texture.h"

#include <flutter_linux/flutter_linux.h>

#include <cstdint>
#include <cstring>
#include <fstream>
#include <map>
#include <memory>
#include <string>
#include <vector>

namespace {

constexpr size_t kShmHeaderSize = 32;
constexpr char kMagic[4] = {'G', 'B', 'V', '1'};

struct ShmHeader {
  uint32_t width;
  uint32_t height;
  uint64_t generation;
};

bool ReadShmHeader(const std::string& path, ShmHeader* out) {
  std::ifstream in(path, std::ios::binary);
  if (!in) {
    return false;
  }
  char magic[4];
  in.read(magic, 4);
  if (!in || std::memcmp(magic, kMagic, 4) != 0) {
    return false;
  }
  uint32_t w = 0;
  uint32_t h = 0;
  uint64_t gen = 0;
  in.read(reinterpret_cast<char*>(&w), 4);
  in.read(reinterpret_cast<char*>(&h), 4);
  in.read(reinterpret_cast<char*>(&gen), 8);
  if (!in || w == 0 || h == 0) {
    return false;
  }
  out->width = w;
  out->height = h;
  out->generation = gen;
  return true;
}

bool ReadShmRgba(const std::string& path, std::vector<uint8_t>* rgba, ShmHeader* hdr) {
  if (!ReadShmHeader(path, hdr)) {
    return false;
  }
  const size_t payload = static_cast<size_t>(hdr->width) * hdr->height * 4;
  std::ifstream in(path, std::ios::binary);
  in.seekg(static_cast<std::streamoff>(kShmHeaderSize), std::ios::beg);
  rgba->resize(payload);
  in.read(reinterpret_cast<char*>(rgba->data()), static_cast<std::streamsize>(payload));
  return static_cast<size_t>(in.gcount()) == payload;
}

struct _CallVideoPixelTexture {
  FlPixelBufferTexture parent_instance;
  std::string shm_path;
  std::vector<uint8_t> rgba;
  uint32_t width;
  uint32_t height;
  uint64_t last_generation;
};

G_DECLARE_FINAL_TYPE(CallVideoPixelTexture, call_video_pixel_texture, GHAL_BOL,
                     CALL_VIDEO_PIXEL_TEXTURE, FlPixelBufferTexture)

G_DEFINE_TYPE(CallVideoPixelTexture, call_video_pixel_texture,
              fl_pixel_buffer_texture_get_type())

static gboolean call_video_pixel_texture_copy_pixels(
    FlPixelBufferTexture* texture, const uint8_t** buffer, uint32_t* width,
    uint32_t* height, GError** /*error*/) {
  CallVideoPixelTexture* self = GHAL_BOL_CALL_VIDEO_PIXEL_TEXTURE(texture);
  ShmHeader hdr{};
  if (!ReadShmRgba(self->shm_path, &self->rgba, &hdr)) {
    return FALSE;
  }
  self->width = hdr.width;
  self->height = hdr.height;
  self->last_generation = hdr.generation;
  *buffer = self->rgba.data();
  *width = self->width;
  *height = self->height;
  return TRUE;
}

static void call_video_pixel_texture_class_init(
    CallVideoPixelTextureClass* klass) {
  FL_PIXEL_BUFFER_TEXTURE_CLASS(klass)->copy_pixels =
      call_video_pixel_texture_copy_pixels;
}

static void call_video_pixel_texture_init(CallVideoPixelTexture* self) {
  self->width = 0;
  self->height = 0;
  self->last_generation = 0;
}

static CallVideoPixelTexture* call_video_pixel_texture_new(
    const std::string& shm_path) {
  CallVideoPixelTexture* self = GHAL_BOL_CALL_VIDEO_PIXEL_TEXTURE(
      g_object_new(call_video_pixel_texture_get_type(), nullptr));
  self->shm_path = shm_path;
  return self;
}

struct TextureEntry {
  FlTextureRegistrar* registrar;
  CallVideoPixelTexture* texture;
  guint timer_id;
  uint64_t last_marked_generation;
};

std::map<int64_t, std::unique_ptr<TextureEntry>> g_textures;

static gboolean texture_poll_cb(gpointer user_data) {
  auto* entry = static_cast<TextureEntry*>(user_data);
  if (entry == nullptr || entry->texture == nullptr) {
    return G_SOURCE_REMOVE;
  }
  ShmHeader hdr{};
  if (!ReadShmHeader(entry->texture->shm_path, &hdr)) {
    return G_SOURCE_CONTINUE;
  }
  if (hdr.generation != entry->last_marked_generation) {
    entry->last_marked_generation = hdr.generation;
    fl_texture_registrar_mark_texture_frame_available(
        entry->registrar, FL_TEXTURE(entry->texture));
  }
  return G_SOURCE_CONTINUE;
}

static void call_video_texture_method_cb(FlMethodChannel* /*channel*/,
                                         FlMethodCall* method_call,
                                         gpointer user_data) {
  FlTextureRegistrar* registrar =
      static_cast<FlTextureRegistrar*>(user_data);
  const gchar* method = fl_method_call_get_name(method_call);
  g_autoptr(GError) error = nullptr;

  if (g_strcmp0(method, "register") == 0) {
    FlValue* args = fl_method_call_get_args(method_call);
    const char* shm_path = nullptr;
    if (fl_value_get_type(args) == FL_VALUE_TYPE_MAP) {
      FlValue* v = fl_value_lookup_string(args, "shmPath");
      if (v != nullptr && fl_value_get_type(v) == FL_VALUE_TYPE_STRING) {
        shm_path = fl_value_get_string(v);
      }
    }
    if (shm_path == nullptr || shm_path[0] == '\0') {
      fl_method_call_respond_error(method_call, "bad_args", "shmPath required",
                                   nullptr, &error);
      return;
    }
    g_autoptr(CallVideoPixelTexture) texture =
        call_video_pixel_texture_new(shm_path);
    if (!fl_texture_registrar_register_texture(registrar,
                                               FL_TEXTURE(texture))) {
      fl_method_call_respond_error(method_call, "register_failed",
                                   "texture register failed", nullptr, &error);
      return;
    }
    const int64_t texture_id = fl_texture_get_id(FL_TEXTURE(texture));
    auto entry = std::make_unique<TextureEntry>();
    entry->registrar = registrar;
    entry->texture = GHAL_BOL_CALL_VIDEO_PIXEL_TEXTURE(g_object_ref(texture));
    entry->last_marked_generation = 0;
    entry->timer_id = g_timeout_add(16, texture_poll_cb, entry.get());
    g_textures.emplace(texture_id, std::move(entry));
    fl_method_call_respond_success(
        method_call, fl_value_new_int(texture_id), &error);
    return;
  }

  if (g_strcmp0(method, "release") == 0) {
    FlValue* args = fl_method_call_get_args(method_call);
    int64_t texture_id = 0;
    if (fl_value_get_type(args) == FL_VALUE_TYPE_MAP) {
      FlValue* v = fl_value_lookup_string(args, "textureId");
      if (v != nullptr && fl_value_get_type(v) == FL_VALUE_TYPE_INT) {
        texture_id = fl_value_get_int(v);
      }
    }
    auto it = g_textures.find(texture_id);
    if (it != g_textures.end()) {
      if (it->second->timer_id != 0) {
        g_source_remove(it->second->timer_id);
      }
      fl_texture_registrar_unregister_texture(it->second->registrar,
                                              FL_TEXTURE(it->second->texture));
      g_object_unref(it->second->texture);
      g_textures.erase(it);
    }
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    return;
  }

  fl_method_call_respond_not_implemented(method_call, &error);
}

}  // namespace

void ghal_bol_register_call_video_texture_channel(FlView* view) {
  FlEngine* engine = fl_view_get_engine(view);
  FlTextureRegistrar* registrar = fl_engine_get_texture_registrar(engine);
  FlBinaryMessenger* messenger = fl_engine_get_binary_messenger(engine);
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  g_autoptr(FlMethodChannel) channel = fl_method_channel_new(
      messenger, "ghal_bol/call_video_texture", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(
      channel, call_video_texture_method_cb, registrar, nullptr);
  g_object_ref(channel);
}
