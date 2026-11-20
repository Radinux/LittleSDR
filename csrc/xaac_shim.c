/* Thin wrapper around libxaac (Apache-2.0) for raw USAC (xHE-AAC) access units:
 * open with an AudioSpecificConfig, decode one access unit at a time to 16-bit PCM. */
#include <stdlib.h>
#include <string.h>
#include "ixheaac_type_def.h"
#include "ixheaac_error_standards.h"
#include "ixheaacd_apicmd_standards.h"
#include "ixheaacd_memory_standards.h"
#include "ixheaacd_aac_config.h"

IA_ERRORCODE ixheaacd_dec_api(pVOID p_ia_module_obj, WORD32 i_cmd, WORD32 i_idx, pVOID pv_value);

#define MAX_MEMS 32
typedef struct {
  void *api;
  void *blocks[MAX_MEMS + 2];
  int nblocks;
  UWORD8 *in;
  WORD32 in_size;
  UWORD8 *out;
} xaac_t;

static void *aligned(xaac_t *x, int size, int align) {
  if (x->nblocks >= MAX_MEMS + 2) return NULL;
  char *p = malloc(size + align);
  if (!p) return NULL;
  x->blocks[x->nblocks++] = p;
  return (void *)(((size_t)p + align - 1) & ~(size_t)(align - 1));
}

void xaac_close(xaac_t *x) {
  if (!x) return;
  for (int i = 0; i < x->nblocks; i++) free(x->blocks[i]);
  free(x);
}

xaac_t *xaac_open(const UWORD8 *asc, int asc_len) {
  xaac_t *x = calloc(1, sizeof(xaac_t));
  WORD32 size = 0, n = 0, v;
  if (!x) return NULL;
  if (ixheaacd_dec_api(NULL, IA_API_CMD_GET_API_SIZE, 0, &size)) goto fail;
  x->api = aligned(x, size, 8);
  if (!x->api || ixheaacd_dec_api(x->api, IA_API_CMD_INIT, IA_CMD_TYPE_INIT_API_PRE_CONFIG_PARAMS, NULL)) goto fail;
  v = 1; /* raw access units, configuration given apart */
  ixheaacd_dec_api(x->api, IA_API_CMD_SET_CONFIG_PARAM, IA_XHEAAC_DEC_CONFIG_PARAM_MP4FLAG, &v);
  v = 16;
  ixheaacd_dec_api(x->api, IA_API_CMD_SET_CONFIG_PARAM, IA_XHEAAC_DEC_CONFIG_PARAM_PCM_WDSZ, &v);
  if (ixheaacd_dec_api(x->api, IA_API_CMD_GET_MEMTABS_SIZE, 0, &size)) goto fail;
  void *tabs = aligned(x, size, 8);
  if (!tabs || ixheaacd_dec_api(x->api, IA_API_CMD_SET_MEMTABS_PTR, 0, tabs)) goto fail;
  if (ixheaacd_dec_api(x->api, IA_API_CMD_INIT, IA_CMD_TYPE_INIT_API_POST_CONFIG_PARAMS, NULL)) goto fail;
  if (ixheaacd_dec_api(x->api, IA_API_CMD_GET_N_MEMTABS, 0, &n) || n > MAX_MEMS) goto fail;
  for (WORD32 i = 0; i < n; i++) {
    WORD32 sz = 0, al = 8, ty = 0;
    ixheaacd_dec_api(x->api, IA_API_CMD_GET_MEM_INFO_SIZE, i, &sz);
    ixheaacd_dec_api(x->api, IA_API_CMD_GET_MEM_INFO_ALIGNMENT, i, &al);
    ixheaacd_dec_api(x->api, IA_API_CMD_GET_MEM_INFO_TYPE, i, &ty);
    void *p = aligned(x, sz, al < 8 ? 8 : al);
    if (!p) goto fail;
    memset(p, 0, sz);
    if (ixheaacd_dec_api(x->api, IA_API_CMD_SET_MEM_PTR, i, p)) goto fail;
    if (ty == IA_MEMTYPE_INPUT) { x->in = p; x->in_size = sz; }
    if (ty == IA_MEMTYPE_OUTPUT) x->out = p;
  }
  if (!x->in || !x->out || asc_len > x->in_size) goto fail;
  /* Initialisation with the configuration */
  memcpy(x->in, asc, asc_len);
  v = asc_len;
  ixheaacd_dec_api(x->api, IA_API_CMD_SET_INPUT_BYTES, 0, &v);
  if (ixheaacd_dec_api(x->api, IA_API_CMD_INIT, IA_CMD_TYPE_INIT_PROCESS, NULL) < 0) goto fail;
  UWORD32 done = 0;
  ixheaacd_dec_api(x->api, IA_API_CMD_INIT, IA_CMD_TYPE_INIT_DONE_QUERY, &done);
  if (!done) goto fail;
  return x;
fail:
  xaac_close(x);
  return NULL;
}

/* One access unit. Returns the number of 16-bit samples written (all channels), or < 0. */
int xaac_decode(xaac_t *x, const UWORD8 *au, int len, WORD16 *pcm, int cap, int *rate, int *channels) {
  if (!x || len > x->in_size) return -1;
  memcpy(x->in, au, len);
  WORD32 v = len;
  ixheaacd_dec_api(x->api, IA_API_CMD_SET_INPUT_BYTES, 0, &v);
  IA_ERRORCODE e = ixheaacd_dec_api(x->api, IA_API_CMD_EXECUTE, IA_CMD_TYPE_DO_EXECUTE, NULL);
  if (e < 0) return -2;
  WORD32 out = 0, sr = 0, ch = 0;
  ixheaacd_dec_api(x->api, IA_API_CMD_GET_OUTPUT_BYTES, 0, &out);
  ixheaacd_dec_api(x->api, IA_API_CMD_GET_CONFIG_PARAM, IA_XHEAAC_DEC_CONFIG_PARAM_SAMP_FREQ, &sr);
  ixheaacd_dec_api(x->api, IA_API_CMD_GET_CONFIG_PARAM, IA_XHEAAC_DEC_CONFIG_PARAM_NUM_CHANNELS, &ch);
  *rate = sr;
  *channels = ch;
  int n = out / 2;
  if (n > cap) n = cap;
  memcpy(pcm, x->out, n * 2);
  return e != 0 && n == 0 ? -3 : n;
}
