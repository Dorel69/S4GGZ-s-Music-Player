# Compactare — 3 noiembrie 2026

Folderul avea **2.588.140.574 octeți** înaintea curățării, aproximativ **2,59 GB / 2,41 GiB** ca dimensiune logică.

Au fost eliminate cache-urile `target/debug`, dependențele și fișierele intermediare din `target/release`, simbolurile PDB, fișierele auxiliare Cargo, directoarele goale de test și executabilul versiunii anterioare. Copiile imaginilor din rădăcină au fost eliminate numai după verificarea egalității lor cu resursele runtime.

Sursele complete, testele, configurația Cargo, launcherul sursă, rapoartele QA și imaginea AVIF sursă nefolosită sunt în **MENTENANTA.zip**, 351.485 octeți. Toate cele **50 de fișiere arhivate** au fost verificate prin SHA-256 înaintea eliminării copiilor necomprimate. Pentru dezvoltare, extrage arhiva în rădăcină și copiază `target/release/assets` în `assets`.

Cele șase executabile/DLL-uri statice folosesc compresia transparentă Windows **LZX**. Cele **235.856.128 octeți** ai acestora ocupă fizic **86.261.760 octeți**, o reducere suplimentară de aproximativ **150 MB**. Conținutul nu este schimbat; Windows le citește direct, fără dezarhivare manuală. Nu s-a aplicat compresie cu pierderi muzicii sau imaginilor. Compresia ține de discul NTFS și poate să nu se păstreze după copiere pe alt disc.

Sunt păstrate launcherul, playerul actual cu Shuffle și scalare 360p–4K, ambele DLL-uri Microsoft, toate temele și animațiile, FFmpeg și FFprobe cu licențele, precum și toate cele **8 fișiere** din folderul Music. Căile muzicii sunt aceleași. Setările, biblioteca și favoritele din afara proiectului nu au fost modificate de curățare.

Verificări:

- **31 de fișiere protejate**: toate prezente, toate cu SHA-256 identic după ștergeri și compresie.
- FFmpeg comprimat a generat un MP3 sintetic; FFprobe l-a identificat și FFmpeg l-a decodat integral: **OK**.
- Pornirea prin `S4GGZ.exe` comprimat a deschis o nouă fereastră nativă a playerului; biblioteca și controalele, inclusiv Shuffle, sunt disponibile.
- Testele Rust ale versiunii actuale rămân consemnate în raportul arhivat: **175 trecute, 0 eșuate, 1 ignorat**. Nu au fost recompilate după curățare, pentru a nu recrea cache-urile.
- `SHA256SUMS.txt` rămâne valid: compresia LZX nu schimbă hash-ul executabilelor.

Dimensiunea finală este aproximativ **137 MB pe disc (131 MiB)**, respectiv **287 MB (274 MiB)** logic, inclusiv muzica. Descărcările ulterioare și recompilarea vor putea mări folderul.

Nu folosi `cargo clean` și nu șterge întregul `target`: acesta conține muzica, imaginile și instrumentele necesare aplicației.

## După actualizarea Download și Repetare

Executabilul cu noua coadă de descărcări și controlul dual Shuffle/Repetare este instalat și comprimat LZX. Au trecut **26 de teste relevante**, fără rularea suitei complete. Cele **50 de fișiere de mentenanță actualizate** au fost verificate în arhiva `MENTENANTA.zip`, de **353.744 octeți**. Cele **29 de fișiere protejate** cu muzică, teme, FFmpeg și runtime Microsoft au SHA-256 identic. Cache-urile temporare au fost eliminate din nou.

O sesiune veche a playerului este încă deschisă. Executabilul său, `target/release/s4ggz.previous-active.exe`, a fost păstrat pentru a nu opri redarea; poate fi șters după închiderea acelei sesiuni. Cu această copie temporară, folderul ocupă aproximativ **147 MB pe disc (140 MiB)**; pachetul de bază rămâne aproximativ **137 MB**.

## După corectarea progresului și simplificarea modurilor

Progresul este citit din stdout și stderr, iar procentele necunoscute sunt înlocuite cu un indicator animat. Playerul are doar cele două moduri exclusive **Shuffle / Loop**, cu schimbarea pozițiilor la selectarea butonului mic. Au trecut **18 teste relevante**; arhiva actualizată are **354.908 octeți**, cu 50 de fișiere de sursă/mentenanță verificate. Cele 29 de fișiere protejate au rămas identice. Copiile sesiunilor anterioare au putut fi eliminate, iar cache-urile au fost curățate din nou. Folderul ocupă acum **137.140.587 octeți pe disc**, aproximativ **137 MB / 131 MiB**.

## Actualizarea din 4 noiembrie 2026

Modul activ poate fi oprit printr-un al doilea click. Footerul are spațiu fix și titlu derulant, iar coperțile personale se păstrează în folderul separat `AlbumArt`, lângă launcher; acest folder este date ale utilizatorului și se păstrează la curățare. Au trecut **12 teste relevante**. Cele **53 de fișiere de mentenanță** actualizate au fost verificate în arhiva de **363.317 octeți**, iar cele **32 de fișiere protejate** au rămas identice. Cache-urile au fost eliminate, executabilul nou este comprimat LZX; o copie a sesiunii anterioare poate rămâne temporar până când acea sesiune este închisă. Dimensiunile istorice de mai sus pot crește odată cu melodiile și coperțile alese de utilizator.
