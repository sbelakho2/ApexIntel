#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# gpu_monitor.sh — Continuous GPU utilization monitor for training
# Logs GPU util%, memory, temp, power to a CSV every N seconds.
# Run in a separate tmux pane during training.
#
# Usage:
#   bash training/gpu_monitor.sh                    # default: every 10s
#   bash training/gpu_monitor.sh 5                  # every 5 seconds
#   bash training/gpu_monitor.sh 10 gpu_log.csv     # custom output file
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

INTERVAL="${1:-10}"
LOGFILE="${2:-/workspace/ApexIntel/training/outputs/logs/gpu_utilization.csv}"
mkdir -p "$(dirname "$LOGFILE")"

echo "GPU Monitor — logging every ${INTERVAL}s to $LOGFILE"
echo "Press Ctrl+C to stop"
echo ""

# Header
echo "timestamp,gpu_id,gpu_name,util_pct,mem_used_mb,mem_total_mb,mem_pct,temp_c,power_w,power_max_w" > "$LOGFILE"

# Alert thresholds
LOW_UTIL_THRESHOLD=50
HIGH_TEMP_THRESHOLD=85

iteration=0
while true; do
    ts=$(date +%Y-%m-%dT%H:%M:%S)
    
    # Get GPU stats
    gpu_data=$(nvidia-smi --query-gpu=index,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw,power.limit \
        --format=csv,noheader,nounits 2>/dev/null || echo "")
    
    if [ -z "$gpu_data" ]; then
        echo "[$ts] ERROR: nvidia-smi failed"
        sleep "$INTERVAL"
        continue
    fi
    
    total_util=0
    total_mem_used=0
    total_mem_total=0
    gpu_count=0
    alerts=""
    
    while IFS=', ' read -r idx name util mem_used mem_total temp power power_max; do
        # Clean whitespace
        idx=$(echo "$idx" | xargs)
        name=$(echo "$name" | xargs)
        util=$(echo "$util" | xargs)
        mem_used=$(echo "$mem_used" | xargs)
        mem_total=$(echo "$mem_total" | xargs)
        temp=$(echo "$temp" | xargs)
        power=$(echo "$power" | xargs)
        power_max=$(echo "$power_max" | xargs)
        
        mem_pct=$( echo "scale=1; $mem_used * 100 / $mem_total" | bc 2>/dev/null || echo "0" )
        
        echo "$ts,$idx,$name,$util,$mem_used,$mem_total,$mem_pct,$temp,$power,$power_max" >> "$LOGFILE"
        
        total_util=$((total_util + ${util%.*}))
        total_mem_used=$((total_mem_used + ${mem_used%.*}))
        total_mem_total=$((total_mem_total + ${mem_total%.*}))
        gpu_count=$((gpu_count + 1))
        
        # Check for low utilization (only after warmup period)
        if [ "$iteration" -gt 5 ] && [ "${util%.*}" -lt "$LOW_UTIL_THRESHOLD" ]; then
            alerts="${alerts}  ⚠  GPU $idx util=${util}% (low)\n"
        fi
        
        # Check for high temperature
        if [ "${temp%.*}" -gt "$HIGH_TEMP_THRESHOLD" ]; then
            alerts="${alerts}  🔥 GPU $idx temp=${temp}°C (high)\n"
        fi
        
    done <<< "$gpu_data"
    
    # Summary line every iteration
    if [ "$gpu_count" -gt 0 ]; then
        avg_util=$((total_util / gpu_count))
        total_mem_gb=$((total_mem_used / 1024))
        max_mem_gb=$((total_mem_total / 1024))
        printf "[%s] %d GPUs | avg util: %3d%% | VRAM: %dGB/%dGB" \
            "$ts" "$gpu_count" "$avg_util" "$total_mem_gb" "$max_mem_gb"
        
        # Color indicator
        if [ "$avg_util" -ge 90 ]; then
            echo " ████ FULL"
        elif [ "$avg_util" -ge 70 ]; then
            echo " ███░ GOOD"
        elif [ "$avg_util" -ge 50 ]; then
            echo " ██░░ MODERATE"
        elif [ "$avg_util" -ge 20 ]; then
            echo " █░░░ LOW"
        else
            echo " ░░░░ IDLE"
        fi
        
        if [ -n "$alerts" ]; then
            printf "%b" "$alerts"
        fi
    fi
    
    iteration=$((iteration + 1))
    sleep "$INTERVAL"
done
