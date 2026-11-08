<p align="center">
  <img src="assets/deskora-logo.png" alt="Deskora" width="440">
</p>

# Deskora

Un viewer VNC nativo per **Windows**, scritto in **Rust**. Interfaccia moderna ed essenziale, connessioni dirette senza account e più spazio al desktop remoto. Rotella regolabile e preset Mac per compensare lo scorrimento lento.

Deskora implementa il protocollo RFB/VNC e funziona senza altri viewer installati. Sul computer remoto deve essere attivo un server VNC compatibile, raggiungibile dalla tua rete o VPN.

## Perché scegliere Deskora

- **Installazione pulita o versione portabile.** Un installer per utente, senza diritti di amministratore, con icona sul desktop facoltativa; oppure un solo `.exe` portabile. Nessun altro client VNC o runtime Python/Node da installare.
- **Connessioni dirette, senza account.** Usa il server VNC che hai già nella tua rete o VPN, senza registrazioni o servizi cloud Deskora.
- **Interfaccia essenziale, in italiano.** Indirizzo, password e Connetti. Impostazioni raccolte in Opzioni e pulsanti chiaramente visibili.
- **Più spazio al desktop remoto.** Una sola barra durante la sessione, schermo intero e tre modalità di visualizzazione: **Riempi**, **Adatta** e **100%**.
- **Una rotella che puoi regolare.** Velocità modificabile durante la sessione e **Preset Mac** dedicato a compensare lo scorrimento troppo lento con Condivisione schermo.
- **Qualità adattabile alla connessione.** Scegli tra **Nitida**, **Bilanciata** e **Fluida** in base alle tue esigenze. Le codifiche effettivamente usate dipendono dal server.
- **Recenti a un clic.** Le ultime cinque connessioni sono in un elenco con indirizzo e porta. Un clic seleziona il computer e apre, sotto l'indirizzo, la tendina della password.
- **Preferenze locali, password non salvate.** Ritrovi configurazione e indirizzi sul tuo computer; la password VNC non viene scritta nelle preferenze.
- **Controlli utili nella sessione.** Mouse, trascinamento, tastiera Unicode, scorrimento verticale e orizzontale, appunti di testo, sola visione, aggiornamento manuale e Ctrl+Alt+Canc.
- **Aggiornamenti gestiti in background.** La decodifica dello schermo avviene su un thread separato; gli aggiornamenti in attesa vengono accorpati per limitare il lavoro ridondante.
- **Anche connessioni inverse.** La modalità in ascolto riceve una connessione avviata da un server VNC compatibile.

Deskora è pensato per chi ha già un server VNC raggiungibile e cerca un client Windows essenziale per accedere ai propri computer in LAN o VPN.

## Rotella regolabile e preset Mac

Se con Condivisione schermo di macOS devi girare molto la rotella per spostarti di poco, puoi adattare lo scorrimento direttamente da Deskora:

1. Durante la sessione, apri **Opzioni**.
2. Premi **Preset Mac** per impostare la velocità a **20×**.
3. Regola il cursore o il campo numerico da **0,5× a 32×**, fino a trovare il valore adatto.

La regolazione agisce nel viewer: puoi compensare lo scorrimento lento senza modificare le impostazioni del Mac. Il risultato dipende dal server e dall'applicazione remota; il preset non garantisce lo stesso comportamento in tutte le app macOS.

## Avvio rapido

1. Installa con **Deskora-Installer.exe** (o usa **Deskora.exe** portabile) e avvia Deskora.
2. Inserisci il nome o l'indirizzo IP del computer remoto e premi **Continua**, oppure scegli un computer tra i **Recenti**.
3. Nella tendina che si apre inserisci la password VNC, se richiesta, e premi **Connetti**.

La porta iniziale è **5900**, modificabile in **Opzioni**. Puoi usare anche `computer::6000` per una porta precisa oppure `computer:2` per il display 2, relativo alla porta base configurata.

**Ctrl+Alt+Maiusc+F** attiva o chiude lo schermo intero. La barra della sessione raccoglie visualizzazione e opzioni.

Gli indirizzi recenti compaiono dopo una connessione riuscita. Preferenze e cronologia si trovano in `%APPDATA%\Deskora\preferences.json`, così la cartella del programma e il desktop restano puliti. Per la modalità portabile crea un `preferences.json` (anche vuoto) accanto a `Deskora.exe`: se è presente, Deskora usa quello.

**Opzioni → Connessione in ascolto** riceve sessioni avviate dal server remoto; porta iniziale **5500**.

## Compatibilità e limiti

- RFB 3.3, 3.7 e 3.8; accesso senza autenticazione, password VNC classica e negoziazione Tight.
- Codifiche Raw, CopyRect, RRE, Hextile, zlib, Tight e ZRLE; cursore remoto e variazioni della risoluzione.
- Tastiera Unicode e appunti di testo RFB classici Latin-1. Negli appunti i caratteri fuori da Latin-1 diventano `?`.
- **VNC classico non cifra il traffico:** usa una rete fidata o una VPN.
- Nessun trasferimento file, audio, relay cloud o supporto RDP/H.264. La fluidità dipende anche dal server e dalla rete; non sono garantiti 60 fps.
- Il preset Mac regola lo scorrimento; la resa nelle singole applicazioni macOS richiede una verifica sul proprio server.

## Compilare

Servono Rust con supporto all'edizione 2024, toolchain MSVC e Windows SDK (Build Tools di Visual Studio, carico di lavoro C++).

```powershell
cargo build --locked --release
```

Il programma compilato è `target/release/deskora.exe`. Logo e icone sono incorporati nell'eseguibile.

Per creare installer e pacchetto portabile, inclusi gli avvisi delle dipendenze:

```powershell
./scripts/package-windows.ps1
```

Il risultato è `dist/Deskora-Installer.exe` (serve [Inno Setup 6](https://jrsoftware.org/isinfo.php); senza, viene saltato con un avviso) e `dist/Deskora-Windows.zip`. Lo script dell'installer è `installer/deskora.iss`: installa per l'utente corrente in `%LOCALAPPDATA%\Programs\Deskora`, crea il collegamento nel menu Start e, se selezionata la casella, quello sul desktop. Build, pacchetti, preferenze e screenshot locali sono esclusi dal repository. `Cargo.lock` e gli asset necessari sono inclusi.

## Release automatiche su GitHub

Il workflow [Windows Release](.github/workflows/release.yml) si attiva quando viene pubblicato un tag `v*`. Compila su Windows x64, verifica formattazione, test e Clippy, quindi crea la release con **Deskora-Installer.exe**, **Deskora-Windows.zip**, i relativi checksum SHA-256 e le note generate da GitHub.

Per la prima release, dopo aver fatto commit e push del workflow e del codice:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

Il tag deve corrispondere alla versione in `Cargo.toml`. Per le release successive aggiorna la versione, esegui `cargo check` per aggiornare `Cargo.lock`, fai commit e push, poi pubblica il nuovo tag, ad esempio `v0.1.1`. I tag con un suffisso, come `v0.2.0-rc.1`, vengono pubblicati come pre-release.

Un normale push del codice non crea release. L'esito della compilazione si trova in **Actions → Windows Release**; a completamento riuscito, il pacchetto compare in **Releases**. La visibilità dei download segue quella del repository: in un repository privato possono scaricarli solo gli utenti autorizzati.

## Verificare

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
cargo run --locked --example connection_check
cargo run --locked --example connection_check -- --compact
```

La fixture apre il viewer reale, usa un server RFB locale simulato e salva una schermata in `shots/`. Non richiede un computer remoto.

Per generare schermate senza caricare la cronologia personale:

```powershell
./target/debug/deskora.exe --screenshot shots/home.png
./target/debug/deskora.exe --screenshot-size 420 460 --screenshot shots/compact.png
./target/debug/deskora.exe --show-options --screenshot shots/options.png
./target/debug/deskora.exe --show-help --screenshot shots/help.png
./target/debug/deskora.exe --sample-recents --show-password --screenshot shots/password.png
```

## Struttura

- `src/main.rs`: finestra, stile e stato dell'applicazione.
- `src/home.rs`: schermata iniziale, tendina della password e recenti.
- `src/desktop.rs`: desktop remoto e barra della sessione.
- `src/session_options.rs`: opzioni della sessione.
- `src/rfb.rs`: protocollo e connessione VNC.
- `src/input.rs`: tastiera e mouse.
- `assets/`: logo, icona e risorse Windows.
- `installer/deskora.iss`: installer Windows (Inno Setup).
- `examples/`: fixture locale e profilazione opzionale.
- `THIRD_PARTY_NOTICES.md` e `licenses/`: avvisi e testi delle dipendenze.
