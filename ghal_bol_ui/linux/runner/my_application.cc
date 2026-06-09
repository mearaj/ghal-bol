#include "my_application.h"
#include "call_video_texture.h"

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif
#include <gdk-pixbuf/gdk-pixbuf.h>
#include <libnotify/notify.h>

#include "flutter/generated_plugin_registrant.h"

static void set_window_icon(GtkWindow* window) {
  g_autofree gchar* exe = g_file_read_link("/proc/self/exe", nullptr);
  if (exe == nullptr) {
    return;
  }
  g_autofree gchar* exe_dir = g_path_get_dirname(exe);
  g_autofree gchar* icon_path =
      g_build_filename(exe_dir, "data", "icons", "app_icon.png", nullptr);
  GError* error = nullptr;
  GdkPixbuf* pixbuf = gdk_pixbuf_new_from_file(icon_path, &error);
  if (pixbuf == nullptr) {
    if (error != nullptr) {
      g_error_free(error);
    }
    return;
  }
  gtk_window_set_icon(window, pixbuf);
  g_object_unref(pixbuf);
}

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  GtkWindow* main_window;
  FlBinaryMessenger* messenger;
  FlMethodChannel* incoming_call_channel;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

static NotifyNotification* incoming_call_notification = nullptr;

static void dismiss_incoming_call_notification();
static void present_main_window(MyApplication* self);

static gboolean quit_application_idle(gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  g_application_quit(G_APPLICATION(self));
  return G_SOURCE_REMOVE;
}

static void notify_dart_method(MyApplication* self, const char* method) {
  if (self->messenger == nullptr) {
    return;
  }
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  g_autoptr(FlValue) args = fl_value_new_null();
  GError* error = nullptr;
  g_autoptr(GBytes) message =
      FL_METHOD_CODEC_GET_CLASS(codec)->encode_method_call(
          FL_METHOD_CODEC(codec), method, args, &error);
  if (message == nullptr) {
    if (error != nullptr) {
      g_error_free(error);
    }
    return;
  }
  fl_binary_messenger_send_on_channel(
      self->messenger, "ghal_bol/incoming_call", message, nullptr, nullptr,
      nullptr);
}

static void notify_dart_opened(MyApplication* self) {
  notify_dart_method(self, "openedFromNotification");
}

static void focus_app_and_notify_dart(MyApplication* self) {
  present_main_window(self);
  notify_dart_opened(self);
}

typedef struct {
  MyApplication* app;
} FocusNotifyIdle;

static gboolean focus_app_and_notify_dart_idle(gpointer user_data) {
  FocusNotifyIdle* data = static_cast<FocusNotifyIdle*>(user_data);
  focus_app_and_notify_dart(data->app);
  g_free(data);
  return G_SOURCE_REMOVE;
}

static void schedule_focus_app_and_notify_dart(MyApplication* self) {
  FocusNotifyIdle* data = g_new(FocusNotifyIdle, 1);
  data->app = self;
  g_idle_add(focus_app_and_notify_dart_idle, data);
}

static gboolean window_is_visible(MyApplication* self) {
  return self->main_window != nullptr &&
         gtk_widget_get_visible(GTK_WIDGET(self->main_window));
}

static void present_main_window(MyApplication* self) {
  if (self->main_window == nullptr) {
    return;
  }
  gtk_widget_show(GTK_WIDGET(self->main_window));
  gtk_window_present(self->main_window);
}

static void incoming_call_notification_action(NotifyNotification* n,
                                              gchar* action,
                                              gpointer user_data) {
  (void)n;
  (void)action;
  schedule_focus_app_and_notify_dart(MY_APPLICATION(user_data));
}

static void incoming_call_notification_activated(NotifyNotification* n,
                                                   gpointer user_data) {
  (void)n;
  schedule_focus_app_and_notify_dart(MY_APPLICATION(user_data));
}

static void incoming_call_notification_closed(NotifyNotification* n,
                                              gpointer user_data) {
  (void)n;
  (void)user_data;
  g_clear_object(&incoming_call_notification);
}

static void show_incoming_call_notification(MyApplication* self,
                                            const char* display_name) {
  if (!notify_is_initted()) {
    notify_init(APPLICATION_ID);
  }
  dismiss_incoming_call_notification();
  incoming_call_notification = notify_notification_new(
      "Incoming call", display_name, "phone-symbolic");
  notify_notification_set_hint(incoming_call_notification, "desktop-entry",
                               g_variant_new_string(APPLICATION_ID));
  notify_notification_set_urgency(incoming_call_notification,
                                  NOTIFY_URGENCY_CRITICAL);
  notify_notification_set_timeout(incoming_call_notification,
                                  NOTIFY_EXPIRES_NEVER);
  g_signal_connect(incoming_call_notification, "closed",
                   G_CALLBACK(incoming_call_notification_closed), self);
  g_signal_connect(incoming_call_notification, "activated",
                   G_CALLBACK(incoming_call_notification_activated), self);
  notify_notification_add_action(incoming_call_notification, "default",
                                 "Open Ghal Bol",
                                 incoming_call_notification_action, self,
                                 nullptr);
  GError* error = nullptr;
  notify_notification_show(incoming_call_notification, &error);
  if (error != nullptr) {
    g_error_free(error);
    g_clear_object(&incoming_call_notification);
  }
}

static void dismiss_incoming_call_notification() {
  if (incoming_call_notification == nullptr) {
    return;
  }
  notify_notification_close(incoming_call_notification, nullptr);
  g_clear_object(&incoming_call_notification);
}

static void incoming_call_method_cb(FlMethodChannel* channel,
                                    FlMethodCall* method_call,
                                    gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  const gchar* method = fl_method_call_get_name(method_call);
  g_autoptr(GError) error = nullptr;

  if (g_strcmp0(method, "show") == 0) {
    const char* name = "Contact";
    FlValue* args = fl_method_call_get_args(method_call);
    if (fl_value_get_type(args) == FL_VALUE_TYPE_MAP) {
      FlValue* v = fl_value_lookup_string(args, "displayName");
      if (v != nullptr && fl_value_get_type(v) == FL_VALUE_TYPE_STRING) {
        name = fl_value_get_string(v);
      }
    }
    show_incoming_call_notification(self, name);
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    return;
  }
  if (g_strcmp0(method, "present") == 0) {
    present_main_window(self);
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    return;
  }
  if (g_strcmp0(method, "openedFromNotification") == 0) {
    focus_app_and_notify_dart(self);
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    return;
  }
  if (g_strcmp0(method, "isWindowVisible") == 0) {
    fl_method_call_respond_success(
        method_call, fl_value_new_bool(window_is_visible(self)), &error);
    return;
  }
  if (g_strcmp0(method, "dismiss") == 0) {
    dismiss_incoming_call_notification();
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    return;
  }
  if (g_strcmp0(method, "hideWindow") == 0) {
    if (self->main_window != nullptr) {
      gtk_widget_hide(GTK_WIDGET(self->main_window));
    }
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    return;
  }
  if (g_strcmp0(method, "quitApplication") == 0) {
    fl_method_call_respond_success(method_call, fl_value_new_null(), &error);
    g_idle_add_full(G_PRIORITY_DEFAULT, quit_application_idle,
                    g_object_ref(self), g_object_unref);
    return;
  }
  fl_method_call_respond_not_implemented(method_call, &error);
}

static void register_incoming_call_channel(MyApplication* self, FlView* view) {
  FlBinaryMessenger* messenger =
      fl_engine_get_binary_messenger(fl_view_get_engine(view));
  self->messenger = messenger;
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->incoming_call_channel = fl_method_channel_new(
      messenger, "ghal_bol/incoming_call", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(
      self->incoming_call_channel, incoming_call_method_cb, g_object_ref(self),
      g_object_unref);
  g_object_ref(self->incoming_call_channel);
}

static gboolean on_window_delete_event(GtkWidget* widget, GdkEvent* event,
                                       gpointer user_data) {
  (void)widget;
  (void)event;
  MyApplication* self = MY_APPLICATION(user_data);
  // Dart ends any active call + clears chat room, then invokes hideWindow.
  notify_dart_method(self, "windowClosedByUser");
  return TRUE;
}

static void first_frame_cb(MyApplication* self, FlView* view) {
  gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  if (self->main_window != nullptr) {
    const bool was_hidden =
        !gtk_widget_get_visible(GTK_WIDGET(self->main_window));
    present_main_window(self);
    if (was_hidden) {
      notify_dart_opened(self);
    }
    return;
  }
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  self->main_window = window;

  gboolean use_header_bar = TRUE;
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    const gchar* wm_name = gdk_x11_screen_get_window_manager_name(screen);
    if (g_strcmp0(wm_name, "GNOME Shell") != 0) {
      use_header_bar = FALSE;
    }
  }
#endif
  if (use_header_bar) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "Ghal Bol");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "Ghal Bol");
  }

  gtk_window_set_default_size(window, 1280, 720);
  set_window_icon(window);
  g_signal_connect(window, "delete-event", G_CALLBACK(on_window_delete_event),
                   self);

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  GdkRGBA background_color;
  gdk_rgba_parse(&background_color, "#000000");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));
  register_incoming_call_channel(self, view);
  ghal_bol_register_call_video_texture_channel(view);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

static void my_application_startup(GApplication* application) {
  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

static void my_application_shutdown(GApplication* application) {
  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  dismiss_incoming_call_notification();
  g_clear_object(&self->incoming_call_channel);
  self->messenger = nullptr;
  self->main_window = nullptr;
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  g_set_prgname(APPLICATION_ID);

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID,
                                     nullptr));
}
