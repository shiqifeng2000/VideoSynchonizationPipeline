cur_dir=$(cd "$(dirname "$0")"; pwd)

mv /data/workspace/glue.tar ./output/window/
cd ./output/window/
tar -xvf glue.tar
rm -rf glue.tar

cd $cur_dir
