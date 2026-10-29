{{- define "zyvor.image" -}}{{ .Values.image.repository }}:{{ .Values.image.tag }}{{- end -}}
{{- define "zyvor.labels" -}}
app.kubernetes.io/name: zyvor
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end -}}
{{- define "zyvor.secEnv" -}}
- name: DATABASE_URL
  valueFrom: { secretKeyRef: { name: {{ .Values.existingSecret }}, key: database-url } }
- name: ZYVOR_MASTER_KEY
  valueFrom: { secretKeyRef: { name: {{ .Values.existingSecret }}, key: master-key } }
{{- end -}}
{{- define "zyvor.podSecurity" -}}
securityContext: { runAsNonRoot: true, runAsUser: 10001, seccompProfile: { type: RuntimeDefault } }
{{- end -}}
{{- define "zyvor.containerSecurity" -}}
securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: ["ALL"] } }
{{- end -}}
