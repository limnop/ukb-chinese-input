LOCAL_PATH := $(call my-dir)

include $(CLEAR_VARS)

LOCAL_C_INCLUDES += $(LOCAL_PATH)/cdict/libcdict
LOCAL_CFLAGS +=
# -Wall -Wextra -Werror
LOCAL_SRC_FILES := cdict/libcdict/libcdict.c cdict/java/jni/juloo_cdict_Cdict.c
LOCAL_MODULE := libcdict_java
LOCAL_SDK_VERSION := 21

# Android requires 16K pages
LOCAL_LDFLAGS += -Wl,-z,max-page-size=16384
# Disable build id to ensure reproducibility, needed for F-Droid
LOCAL_LDFLAGS += -Wl,--build-id=none

include $(BUILD_SHARED_LIBRARY)

include $(CLEAR_VARS)
LOCAL_MODULE := wowtype_core
LOCAL_SRC_FILES := ../build/wowtype-rust/$(if $(filter arm64-v8a,$(TARGET_ARCH_ABI)),aarch64-linux-android,$(if $(filter armeabi-v7a,$(TARGET_ARCH_ABI)),armv7-linux-androideabi,$(if $(filter x86,$(TARGET_ARCH_ABI)),i686-linux-android,x86_64-linux-android)))/release/libwowtype_core.a
include $(PREBUILT_STATIC_LIBRARY)

include $(CLEAR_VARS)
LOCAL_MODULE := wowtype_jni
LOCAL_SRC_FILES := wowtype-jni/wowtype_jni.c
LOCAL_C_INCLUDES += $(LOCAL_PATH)/wowtype-core/include
LOCAL_WHOLE_STATIC_LIBRARIES += wowtype_core
LOCAL_LDLIBS += -llog -ldl -lm
LOCAL_LDFLAGS += -Wl,-z,max-page-size=16384
LOCAL_LDFLAGS += -Wl,--build-id=none
include $(BUILD_SHARED_LIBRARY)
