

#!/bin/sh

cur_dir=$(cd "$(dirname "$0")"; pwd)
dst_dir=output/linux
so_path=./target/release/libglue.so

cargo build --release
cp $so_path $dst_dir
cd $dst_dir

# gcc test.c -o test -I./ -L./ -lglue
gcc test.c -o test -I./ -L./ -lglue -Wl,-rpath,/data/workspace/boe/2026_heterogeneous_space_aigc/output/linux

chmod +x ./test
# cp ./test $cur_dir

cd $cur_dir

# LD_LIBRARY_PATH=./ 
RUST_BACKTRACE=1 $dst_dir/test '/data/workspace/boe/2026_heterogeneous_space_aigc/media' 0 0


# sudo rm -rf dst \
#     && mkdir dst \
#     && docker run -v /data/workspace/boe/vccplayer/src:/app/src -v /data/workspace/boe/vccplayer/build.rs:/app/build.rs -v /data/workspace/boe/vccplayer/Cargo.toml:/app/Cargo.toml -v /data/workspace/boe/vccplayer/Cargo.lock:/app/Cargo.lock -v /data/workspace/boe/vccplayer/dst:/app/target -t vccplayer:v5.0 /root/.cargo/bin/cargo build --release \
#     && cp dst/release/vccplayer ./ \
#     && sudo rm -rf dst
# cd ../../../ && cargo build --release && cp ./target/release/libdatabase.so ./bin/file_search/x64/ && cd bin/file_search/x64  && gcc ./demo.cc -o demo -L./ -ldatabase && LD_LIBRARY_PATH=./ ./demo ./config.yml ../text/