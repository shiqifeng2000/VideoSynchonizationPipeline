// test_glue.c
#include "glue.h"
#include <stdio.h>

const int WIDTH  = 1920;
const int HEIGHT = 1080;

int main(int argc, char *argv[]) {
    printf("测试用app, 默认log为日志路径\n");
    if (argc < 4)
    {
        fprintf(stderr, "Usage: %s <media_path> with 4 test videos\n <type>: 0 => video{num}.mp4,1 => video_hevc{num}.mp4,2 => sample{num}.mp4,3 => sample_idx{num}.mp4\n<view_ports> number\n", argv[0]);
        return 1;
    }
    char* media_path = argv[1];
    int type = atoi(argv[2]);
    int view_ports = atoi(argv[3]);
    // int height = atoi(argv[3]);
    // int code = run_image_app(argv[1], width, height, init_image_demo,register_image_demo_texture,draw_image_demo_texture,destroy_image_demo);

    init_logger("./log", 1);
    int code = mock_video_player(type,
                      view_ports,
                      media_path,
                      init_player,
                      get_ratios,
                      register_textures,
                      unregister_textures,
                      start_player,
                      pause_player,
                      load_frames,
                      draw_frames,
                    //   recycle_frames,
                      stop_player,
                      terminate_player);
    return code;
}