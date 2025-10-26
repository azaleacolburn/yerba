#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/time.h>

double get_time() {
  struct timeval t;
  struct timezone tzp;
  gettimeofday(&t, &tzp);
  return t.tv_sec + t.tv_usec * 1e-6;
}
int main() {
  double initial = get_time();
  int *page = malloc(4000);
  double final = get_time();
  printf("Initial: %lf\n", initial);
  printf("Final  : %lf\n", final);
  printf("Elapsed for malloc: %lf\n", final - initial);

  double initial_two = get_time();
  free(page);
  double final_two = get_time();
  printf("Elapsed for free: %lf\n", final_two - initial_two);
}
