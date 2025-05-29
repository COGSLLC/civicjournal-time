/*
 * C API for civicjournal-time
 *
 * This header defines the C interface for the civicjournal-time library.
 */

#ifndef CIVICJOURNAL_TIME_H
#define CIVICJOURNAL_TIME_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to a TimeHierarchy instance */
typedef struct CJTimeHierarchy CJTimeHierarchy;

/* Creates a new TimeHierarchy instance */
CJTimeHierarchy* civicjournal_time_hierarchy_new(void);

/* Frees a TimeHierarchy instance */
void civicjournal_time_hierarchy_free(CJTimeHierarchy* hierarchy);

/* Gets the current time hierarchy statistics as a JSON string */
char* civicjournal_time_hierarchy_stats(const CJTimeHierarchy* hierarchy);

/* Frees a string that was allocated by the library */
void civicjournal_free_string(char* str);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* CIVICJOURNAL_TIME_H */
