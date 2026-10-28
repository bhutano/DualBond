!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Arresto del servizio DualBond esistente"
  nsExec::ExecToLog 'sc.exe stop DualBondService'
  ; sc stop ritorna subito: il processo del servizio tiene ancora bloccato
  ; dualbond.exe per qualche istante e in modalità silenziosa (/S) NSIS
  ; salterebbe la copia del file senza errori, lasciando installata la
  ; versione vecchia. Aspettiamo (max 20 s) che il processo esca davvero.
  StrCpy $R9 0
  dualbond_wait_service_exit:
    nsExec::ExecToStack 'cmd.exe /c tasklist /FI "IMAGENAME eq dualbond.exe" /FI "SESSION eq 0" /NH | find /I "dualbond.exe"'
    Pop $0
    Pop $1
    StrCmp $0 "0" 0 dualbond_service_exited
    IntOp $R9 $R9 + 1
    IntCmp $R9 40 dualbond_service_exited
    Sleep 500
    Goto dualbond_wait_service_exit
  dualbond_service_exited:
  nsExec::ExecToLog 'sc.exe delete DualBondService'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Configurazione del servizio DualBond"
  nsExec::ExecToStack '"$INSTDIR\dualbond.exe" install-service'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "Il servizio DualBond non è stato installato (codice $0). Reinstalla l'app come amministratore."
    Abort
  ${EndIf}

  ; Per impostazione predefinita Windows "riduce al minimo le connessioni
  ; simultanee": quando si collega l'Ethernet stacca il Wi-Fi (evento WLAN
  ; 8003, "criterio che ha disabilitato la connessione automatica"). DualBond
  ; ha bisogno di tutte le connessioni attive insieme, quindi le consente.
  ; Il valore precedente viene salvato una sola volta e ripristinato alla
  ; disinstallazione.
  DetailPrint "Consento a Windows di restare connesso a più reti insieme"
  SetRegView 64
  ClearErrors
  ReadRegStr $2 HKLM "Software\DualBond" "WcmMinimizeConnectionsPrevious"
  ${If} ${Errors}
    ClearErrors
    ReadRegDWORD $2 HKLM "SOFTWARE\Policies\Microsoft\Windows\WcmSvc\GroupPolicy" "fMinimizeConnections"
    ${If} ${Errors}
      StrCpy $2 "absent"
    ${EndIf}
    WriteRegStr HKLM "Software\DualBond" "WcmMinimizeConnectionsPrevious" $2
  ${EndIf}
  WriteRegDWORD HKLM "SOFTWARE\Policies\Microsoft\Windows\WcmSvc\GroupPolicy" "fMinimizeConnections" 0

  ; Se il servizio va in crash senza lasciare traccia nel log (es. abort del
  ; processo) Windows salva un minidump locale: resta sul PC, non viene
  ; inviato da nessuna parte, al massimo 3 file.
  DetailPrint "Abilito i report di crash locali di DualBond"
  WriteRegExpandStr HKLM "SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\dualbond.exe" "DumpFolder" "%ProgramData%\DualBond\crashdumps"
  WriteRegDWORD HKLM "SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\dualbond.exe" "DumpCount" 3
  WriteRegDWORD HKLM "SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\dualbond.exe" "DumpType" 1
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Rimozione del servizio DualBond"
  nsExec::ExecToLog '"$INSTDIR\dualbond.exe" remove-service'
  SetRegView 64
  DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\dualbond.exe"

  DetailPrint "Ripristino della gestione connessioni di Windows"
  SetRegView 64
  ClearErrors
  ReadRegStr $2 HKLM "Software\DualBond" "WcmMinimizeConnectionsPrevious"
  ${IfNot} ${Errors}
    ${If} $2 == "absent"
      DeleteRegValue HKLM "SOFTWARE\Policies\Microsoft\Windows\WcmSvc\GroupPolicy" "fMinimizeConnections"
    ${Else}
      WriteRegDWORD HKLM "SOFTWARE\Policies\Microsoft\Windows\WcmSvc\GroupPolicy" "fMinimizeConnections" $2
    ${EndIf}
    DeleteRegValue HKLM "Software\DualBond" "WcmMinimizeConnectionsPrevious"
    DeleteRegKey /ifempty HKLM "Software\DualBond"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
