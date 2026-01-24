#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct RdpRuntime RdpRuntime;

typedef int32_t RESULT;

typedef struct RdpRuntime *RDP;

/**
 * No error.
 */
#define RESULT_OK 0

/**
 * Unknown error.
 */
#define RESULT_ERR_UNKNOWN -1

/**
 * Utf8 error.
 */
#define RESULT_ERR_UTF8 -2

/**
 * The other side is closed.
 */
#define RESULT_ERR_CLOSED -3

void rdp_setup_stdout_logger(void);

RESULT rdp_run(RDP *rabbit_digger, const char *config);

RESULT rdp_update_config(RDP rabbit_digger, const char *config);

RESULT rdp_stop(RDP *rabbit_digger);
