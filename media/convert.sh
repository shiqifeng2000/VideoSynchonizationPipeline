#!/bin/bash

# RGBA to PNG 批量转换脚本
# 用法: ./convert_rgba_to_png.sh [宽度] [高度]

# 设置默认分辨率（如果没有提供参数）
WIDTH=${1:-1920}    # 默认宽度 1920
HEIGHT=${2:-1080}   # 默认高度 1080

echo "开始转换 RGBA 文件到 PNG 格式..."
echo "分辨率: ${WIDTH}x${HEIGHT}"
echo "======================================"

# 计数器
converted_count=0
skipped_count=0
error_count=0

# 查找所有可能的RGBA文件（支持常见扩展名）
for file in *.rgba *.raw *.data; do
    # 确保文件存在（避免在没有匹配文件时执行）
    [ -e "$file" ] || continue
    
    # 生成输出文件名
    output_file="${file%.*}.png"

    # 如果PNG文件已存在，跳过
    if [ -f "$output_file" ]; then
        echo "跳过: $output_file 已存在"
        ((skipped_count++))
        continue
    fi
    
    echo "正在转换: $file -> $output_file"
    
    # 使用ffmpeg进行转换
    # -pixel_format argb 指定像素格式（常见的RGBA格式）
    # -video_size 指定分辨率
    # -i 输入文件
    if ffmpeg -v error -f rawvideo -pixel_format argb \
        -video_size "${WIDTH}x${HEIGHT}" \
        -i "$file" \
        "$output_file"; then
        echo "✓ 成功转换: $output_file"
        ((converted_count++))
    else
        echo "✗ 转换失败: $file"
        ((error_count++))
        # 删除可能生成的不完整文件
        [ -f "$output_file" ] && rm "$output_file"
    fi
done

for file in *.nv12; do
    # 确保文件存在（避免在没有匹配文件时执行）
    [ -e "$file" ] || continue
    
    # 生成输出文件名
    output_file="${file%.*}.jpg"

    # 如果PNG文件已存在，跳过
    if [ -f "$output_file" ]; then
        echo "跳过: $output_file 已存在"
        ((skipped_count++))
        continue
    fi
    
    echo "正在转换: $file -> $output_file"
    
    # 使用ffmpeg进行转换
    # -pixel_format argb 指定像素格式（常见的RGBA格式）
    # -video_size 指定分辨率
    # -i 输入文件
    if ffmpeg -v error -f rawvideo -pixel_format nv12 \
        -video_size "${WIDTH}x${HEIGHT}" \
        -i "$file" \
        "$output_file"; then
        echo "✓ 成功转换: $output_file"
        ((converted_count++))
    else
        echo "✗ 转换失败: $file"
        ((error_count++))
        # 删除可能生成的不完整文件
        [ -f "$output_file" ] && rm "$output_file"
    fi
done

echo "======================================"
echo "转换完成!"
echo "成功: $converted_count 个文件"
echo "跳过: $skipped_count 个文件"
echo "失败: $error_count 个文件"

# 如果没有找到任何RGBA文件
if [ $converted_count -eq 0 ] && [ $skipped_count -eq 0 ] && [ $error_count -eq 0 ]; then
    echo "警告: 没有找到任何RGBA文件 (*.rgba, *.raw, *.data)"
    echo "请确保文件在当前目录中"
fi