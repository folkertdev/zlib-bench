# zlib-bench

Rust program to benchmark zlib inflate and deflate throughput

## Deflate

The compression level is configurable

```
> cargo run --release deflate-all 1 silesia-small.tar
implementation, MB/s
og, 86.91752530268442
ng, 159.6751550929082
rs, 142.73719043986569
cloudflare, 84.13098985203082
miniz, 119.91810369707754

> cargo run --release deflate-all 6 silesia-small.tar
implementation, MB/s
og, 41.3123021727419
ng, 50.15540153152186
rs, 45.34501820183941
cloudflare, 44.672619408724024
miniz, 23.450970096318063

> cargo run --release deflate-all 9 silesia-small.tar
implementation, MB/s
og, 27.601809433148645
ng, 26.28268212871113
rs, 24.552689695096593
cloudflare, 28.935540915654574
miniz, 13.816265857470075
```

## Inflate

```
> cargo run --release inflate-all silesia-small.tar.gz
implementation, MB/s
og, 149.96425609291938
ng, 153.6494515918422
rs, 141.75241369292857
cloudflare, 140.85586537114003
miniz, 99.66201229685869
```
