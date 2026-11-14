#include <jni.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wowtype_core.h"

static const char *get_utf(JNIEnv *env, jstring value)
{
  return value == NULL ? NULL : (*env)->GetStringUTFChars(env, value, NULL);
}

static void release_utf(JNIEnv *env, jstring value, const char *text)
{
  if (value != NULL && text != NULL)
    (*env)->ReleaseStringUTFChars(env, value, text);
}

JNIEXPORT jlong JNICALL
Java_juloo_keyboard2_staged_WowtypePinyinEngine_nativeNew(
    JNIEnv *env, jclass cls, jstring lexicon_path, jstring history_path,
    jstring user_dict_path)
{
  (void)cls;
  const char *lexicon = get_utf(env, lexicon_path);
  const char *history = get_utf(env, history_path);
  const char *user_dict = get_utf(env, user_dict_path);
  if (lexicon == NULL)
    return 0;
  WowtypeEngineHandle *engine = wowtype_engine_new(lexicon, NULL);
  if (engine != NULL) {
    (void)wowtype_engine_set_max_candidates(engine, 100);
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
  WowtypeCandidates *candidates = wowtype_engine_input(engine, code);
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
    jstring item = (*env)->NewStringUTF(env, record);
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
