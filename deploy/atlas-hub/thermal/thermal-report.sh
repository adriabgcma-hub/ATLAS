#!/usr/bin/env bash
# Informe térmico de atlas-hub. Solo lectura, sin privilegios.
set -euo pipefail
SMC=/sys/devices/platform/applesmc.768

echo "atlas-hub · $(date '+%Y-%m-%d %H:%M:%S %Z')"
echo "uptime: $(uptime -p)"
echo "carga:  $(cut -d' ' -f1-3 /proc/loadavg)"
echo

echo "== CPU (coretemp) =="
for d in /sys/class/hwmon/hwmon*; do
  [ "$(cat "$d/name" 2>/dev/null)" = coretemp ] || continue
  for f in "$d"/temp*_input; do
    [ -e "$f" ] || continue
    label=$(cat "${f%_input}_label" 2>/dev/null || basename "$f")
    awk -v l="$label" '{printf "%s: %.1f °C\n", l, $1/1000}' "$f"
  done
done
echo

echo "== PCH =="
for z in /sys/class/thermal/thermal_zone*; do
  t=$(cat "$z/type" 2>/dev/null || true)
  case "$t" in
    pch_*) awk -v l="$t" '{printf "%s: %.1f °C\n", l, $1/1000}' "$z/temp" ;;
  esac
done
echo

echo "== Batería =="
if [ -e /sys/class/thermal/thermal_zone1/temp ]; then
  awk '{printf "zona BAT0: %.1f °C\n", $1/1000}' /sys/class/thermal/thermal_zone1/temp
fi
printf 'estado: %s · carga: %s%% · ciclos: %s\n' \
  "$(cat /sys/class/power_supply/BAT0/status 2>/dev/null || echo ?)" \
  "$(cat /sys/class/power_supply/BAT0/capacity 2>/dev/null || echo ?)" \
  "$(cat /sys/class/power_supply/BAT0/cycle_count 2>/dev/null || echo ?)"
echo

echo "== Ventilador =="
printf 'RPM: %s · modo manual: %s · mín: %s · máx: %s\n' \
  "$(cat "$SMC/fan1_input")" "$(cat "$SMC/fan1_manual")" \
  "$(cat "$SMC/fan1_min")" "$(cat "$SMC/fan1_max")"
echo

echo "== Curva (/etc/mbpfan.conf) =="
grep -E '^(low_temp|high_temp|max_temp|min_fan1_speed|max_fan1_speed|polling_interval)' /etc/mbpfan.conf 2>/dev/null || echo "sin config"
echo

echo "== Límites de CPU =="
for f in /sys/devices/system/cpu/intel_pstate/no_turbo \
         /sys/devices/system/cpu/intel_pstate/max_perf_pct \
         /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor; do
  [ -e "$f" ] && printf '%s = %s\n' "$(basename "$f")" "$(cat "$f")"
done
