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
{{- define "zyvor.mailEnv" -}}
{{- if .Values.mail.enabled }}
- name: ZYVOR_SMTP_URL
  valueFrom: { secretKeyRef: { name: {{ .Values.mail.existingSecret }}, key: smtp-url } }
- name: ZYVOR_MAIL_FROM
  valueFrom: { secretKeyRef: { name: {{ .Values.mail.existingSecret }}, key: from } }
{{- end }}
{{- end -}}
{{- define "zyvor.kycEnv" -}}
{{- if .Values.kyc.enabled }}
- name: ZYVOR_KYC_PROVIDER
  valueFrom: { secretKeyRef: { name: {{ .Values.kyc.existingSecret }}, key: provider } }
- name: ZYVOR_KYC_CASHFREE_ENV
  valueFrom: { secretKeyRef: { name: {{ .Values.kyc.existingSecret }}, key: env, optional: true } }
- name: ZYVOR_KYC_CASHFREE_CLIENT_ID
  valueFrom: { secretKeyRef: { name: {{ .Values.kyc.existingSecret }}, key: client-id, optional: true } }
- name: ZYVOR_KYC_CASHFREE_CLIENT_SECRET
  valueFrom: { secretKeyRef: { name: {{ .Values.kyc.existingSecret }}, key: client-secret, optional: true } }
{{- end }}
{{- end -}}
{{- define "zyvor.podSecurity" -}}
securityContext: { runAsNonRoot: true, runAsUser: 10001, seccompProfile: { type: RuntimeDefault } }
{{- end -}}
{{- define "zyvor.containerSecurity" -}}
securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: ["ALL"] } }
{{- end -}}
