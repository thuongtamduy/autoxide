- Search by meaning is faster on large sites (step 39): the vector comparison uses the CPU's vector instructions and
  the scan is split over the CPU cores outside the request thread — 50,000 posts in one language (100,000 chunks of
  1,024 dimensions) went from 137 ms to 20 ms on 4 cores, with the same results.
