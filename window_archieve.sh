cur_dir=$(cd "$(dirname "$0")"; pwd)
# zip -r agent.zip ./agent ./common ./crates ./Cargo.*
zip -r src.zip ./src

mv src.zip /data/workspace/

cd $cur_dir

