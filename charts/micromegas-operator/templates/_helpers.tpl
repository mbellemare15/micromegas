{{- define "micromegas-operator.name" -}}
{{- default .Chart.Name .Values.nameOverride }}
{{- end }}
{{- define "micromegas-operator.fullname" -}}
{{- if .Values.fullnameOverride }}{{ .Values.fullnameOverride }}{{ else }}
{{- $name := include "micromegas-operator.name" . }}
{{- if contains $name .Release.Name }}{{ .Release.Name }}{{ else }}{{ printf "%s-%s" .Release.Name $name }}{{ end }}
{{- end }}
{{- end }}
{{- define "micromegas-operator.labels" -}}
app.kubernetes.io/name: {{ include "micromegas-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}
{{- define "micromegas-operator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "micromegas-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
