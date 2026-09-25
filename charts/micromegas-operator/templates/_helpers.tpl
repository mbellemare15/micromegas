{{- define "micromegas-operator.name" -}}
{{ .Chart.Name }}
{{- end }}
{{- define "micromegas-operator.fullname" -}}
{{- if contains .Chart.Name .Release.Name }}{{ .Release.Name }}{{ else }}{{ printf "%s-%s" .Release.Name .Chart.Name }}{{ end }}
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
