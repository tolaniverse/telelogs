{{- define "collector.name" -}}
{{- default .Release.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "collector.labels" -}}
app.kubernetes.io/name: telelog-collector
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "collector.selector" -}}
app.kubernetes.io/name: telelog-collector
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "collector.secret" -}}
{{- default (printf "%s-enroll" (include "collector.name" .)) .Values.existingSecret -}}
{{- end -}}
