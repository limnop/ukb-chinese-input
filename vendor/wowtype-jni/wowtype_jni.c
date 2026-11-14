#include <jni.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wowtype_core.h"

/* JNI modified UTF-8 cannot represent supplementary Han characters or emoji.
 * Use Java's standard UTF-8 codec at both boundaries. */
static const char *get_utf(JNIEnv *env, jstring value)
{
  if (value == NULL) return NULL;
  jclass cls = (*env)->FindClass(env, "java/lang/String");
  jmethodID method = (*env)->GetMethodID(env, cls, "getBytes", "(Ljava/lang/String;)[B");
  jstring encoding = (*env)->NewStringUTF(env, "UTF-8");
  jbyteArray bytes = (jbyteArray)(*env)->CallObjectMethod(env, value, method, encoding);
  char *result = NULL;
  if (bytes != NULL) {
    jsize len = (*env)->GetArrayLength(env, bytes);
    result = malloc((size_t)len + 1);
    if (result != NULL) {
      (*env)->GetByteArrayRegion(env, bytes, 0, len, (jbyte *)result);
      result[len] = 0;
    }
    (*env)->DeleteLocalRef(env, bytes);
  }
  (*env)->DeleteLocalRef(env, encoding);
  (*env)->DeleteLocalRef(env, cls);
  return result;
}
static void release_utf(JNIEnv *env, jstring value, const char *text)
{
  (void)env; (void)value; free((void *)text);
}
static jstring new_utf8(JNIEnv *env, const char *text)
{
  jsize len = (jsize)strlen(text);
  jbyteArray bytes = (*env)->NewByteArray(env, len);
  if (bytes == NULL) return NULL;
  (*env)->SetByteArrayRegion(env, bytes, 0, len, (const jbyte *)text);
  jclass cls = (*env)->FindClass(env, "java/lang/String");
  jmethodID constructor = (*env)->GetMethodID(env, cls, "<init>", "([BLjava/lang/String;)V");
  jstring encoding = (*env)->NewStringUTF(env, "UTF-8");
  jstring result = (jstring)(*env)->NewObject(env, cls, constructor, bytes, encoding);
  (*env)->DeleteLocalRef(env, bytes);
  (*env)->DeleteLocalRef(env, encoding);
  (*env)->DeleteLocalRef(env, cls);
  return result;
}

JNIEXPORT jlong JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeNew(
    JNIEnv *env, jclass cls, jstring lexicon_path, jstring model_path, jstring history_path,
    jstring user_dict_path)
{
  (void)cls;
  const char *lexicon = get_utf(env, lexicon_path);
  const char *history = get_utf(env, history_path);
  const char *user_dict = get_utf(env, user_dict_path);
  if (lexicon == NULL)
    return 0;
  const char *model = get_utf(env, model_path);
  WowtypeEngineHandle *engine = wowtype_engine_new(lexicon, model);
  release_utf(env, model_path, model);
  if (engine != NULL) {
    (void)wowtype_engine_set_max_candidates(engine, 1000);
    (void)wowtype_engine_set_beam_size(engine, 20);
    if (history != NULL) (void)wowtype_engine_load_history(engine, history);
    if (user_dict != NULL) (void)wowtype_engine_load_user_dict(engine, user_dict);
  }
  release_utf(env, user_dict_path, user_dict);
  release_utf(env, history_path, history);
  release_utf(env, lexicon_path, lexicon);
  return (jlong)(intptr_t)engine;
}

JNIEXPORT jobjectArray JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeInput(
    JNIEnv *env, jclass cls, jlong raw_engine, jstring input, jint limit)
{
  (void)cls;
  WowtypeEngineHandle *engine = (WowtypeEngineHandle *)(intptr_t)raw_engine;
  const char *code = get_utf(env, input);
  jclass string_class = (*env)->FindClass(env, "java/lang/String");
  if (string_class == NULL) {
    release_utf(env, input, code);
    return NULL;
  }
  if (engine == NULL || code == NULL || code[0] == '\0') {
    jobjectArray empty = (*env)->NewObjectArray(env, 0, string_class, NULL);
    release_utf(env, input, code);
    return empty;
  }
  WowtypeCandidates *candidates = code[0] == 'v'
      ? wowtype_calc_candidates(code + 1) : wowtype_engine_input(engine, code);
  if (candidates == NULL) {
    jobjectArray empty = (*env)->NewObjectArray(env, 0, string_class, NULL);
    release_utf(env, input, code);
    return empty;
  }
  size_t count = wowtype_candidates_len(candidates);
  if (limit >= 0 && count > (size_t)limit) count = (size_t)limit;
  jobjectArray result = (*env)->NewObjectArray(env, (jsize)count, string_class, NULL);
  for (size_t i = 0; i < count && result != NULL; ++i) {
    const char *text = wowtype_candidates_text(candidates, i);
    size_t consumed = wowtype_candidates_consumed(candidates, i);
    char prefix[32];
    snprintf(prefix, sizeof(prefix), "%zu\t", consumed);
    size_t prefix_len = strlen(prefix);
    size_t text_len = text == NULL ? 0 : strlen(text);
    char *record = (char *)malloc(prefix_len + text_len + 1);
    if (record == NULL) continue;
    memcpy(record, prefix, prefix_len);
    if (text_len > 0) memcpy(record + prefix_len, text, text_len);
    record[prefix_len + text_len] = '\0';
    jstring item = new_utf8(env, record);
    if (item != NULL) {
      (*env)->SetObjectArrayElement(env, result, (jsize)i, item);
      (*env)->DeleteLocalRef(env, item);
    }
    free(record);
  }
  wowtype_candidates_free(candidates);
  release_utf(env, input, code);
  return result;
}

JNIEXPORT void JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeConfigure(
    JNIEnv *env, jclass cls, jlong raw_engine, jboolean fuzzy,
    jboolean typo_correction)
{
  (void)env;
  (void)cls;
  WowtypeEngineHandle *engine = (WowtypeEngineHandle *)(intptr_t)raw_engine;
  if (engine == NULL) return;
  (void)wowtype_engine_set_fuzzy(engine, fuzzy ? 1 : 0);
  (void)wowtype_engine_set_typo_correction(engine, typo_correction ? 1 : 0);
}

JNIEXPORT void JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeSetContext(
    JNIEnv *env, jclass cls, jlong raw_engine, jstring context)
{
  (void)cls;
  WowtypeEngineHandle *engine = (WowtypeEngineHandle *)(intptr_t)raw_engine;
  const char *text = get_utf(env, context);
  if (engine != NULL && text != NULL)
    (void)wowtype_engine_set_context(engine, text);
  release_utf(env, context, text);
}

JNIEXPORT void JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeRecordSelection(
    JNIEnv *env, jclass cls, jlong raw_engine, jstring word, jstring pinyin,
    jstring history_path, jstring user_dict_path)
{
  (void)cls;
  WowtypeEngineHandle *engine = (WowtypeEngineHandle *)(intptr_t)raw_engine;
  const char *word_text = get_utf(env, word);
  const char *pinyin_text = get_utf(env, pinyin);
  const char *history = get_utf(env, history_path);
  const char *user_dict = get_utf(env, user_dict_path);
  if (engine != NULL && word_text != NULL) {
    /* Learn the decoder's full pinyin, never a typo or abbreviated raw code. */
    WowtypeCandidates *choices = pinyin_text == NULL ? NULL
        : wowtype_engine_input(engine, pinyin_text);
    const char *reading = NULL;
    if (choices != NULL) {
      for (size_t i = 0; i < wowtype_candidates_len(choices); ++i) {
        const char *candidate = wowtype_candidates_text(choices, i);
        if (candidate != NULL && strcmp(candidate, word_text) == 0) {
          reading = wowtype_candidates_pinyin(choices, i);
          break;
        }
      }
    }
    (void)wowtype_engine_record_selection_ex(engine, word_text, reading);
    if (choices != NULL) wowtype_candidates_free(choices);
    if (history != NULL) (void)wowtype_engine_save_history(engine, history);
    if (user_dict != NULL) (void)wowtype_engine_save_user_dict(engine, user_dict);
  }
  release_utf(env, user_dict_path, user_dict);
  release_utf(env, history_path, history);
  release_utf(env, pinyin, pinyin_text);
  release_utf(env, word, word_text);
}

JNIEXPORT void JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeFree(
    JNIEnv *env, jclass cls, jlong raw_engine)
{
  (void)env;
  (void)cls;
  wowtype_engine_free((WowtypeEngineHandle *)(intptr_t)raw_engine);
}
