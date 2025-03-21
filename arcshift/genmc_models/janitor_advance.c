/* file: mp.c */
#include <stdlib.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <assert.h>

atomic_int weak1;
atomic_int next1;
atomic_int advance1;
atomic_int weak2;
atomic_int weak3;
int payload = 1;

void *thread_1(void *unused)
{
    atomic_fetch_add_explicit(&advance1, 1, memory_order_seq_cst);
    int next1_value = atomic_load_explicit(&next1, memory_order_seq_cst);
    if (next1_value==1)
    {
        atomic_fetch_add_explicit(&weak2, 1, memory_order_release);
        atomic_fetch_sub_explicit(&advance1, 1, memory_order_release);
        atomic_fetch_sub_explicit(&weak1, 1, memory_order_relaxed);
        assert(payload==1);
    } else {
        atomic_fetch_add_explicit(&weak3, 1, memory_order_relaxed);

    }
    return NULL;
}

void *thread_2(void *unused)
{
    atomic_store_explicit(&next1, 2, memory_order_release);
    atomic_thread_fence(memory_order_seq_cst);
    if (atomic_load_explicit(&advance1, memory_order_acquire)==0) {
        if (atomic_load_explicit(&weak2, memory_order_acquire)==0) {
            payload=0;
        }
    }
    return NULL;
}


int main()
{
    weak1 = 1;
    weak2 = 0;
    advance1 = 0;
    next1 = 1;
    pthread_t t1, t2;

    if (pthread_create(&t1, NULL, thread_1, NULL))
            abort();
    if (pthread_create(&t2, NULL, thread_2, NULL))
            abort();

    return 0;
}
