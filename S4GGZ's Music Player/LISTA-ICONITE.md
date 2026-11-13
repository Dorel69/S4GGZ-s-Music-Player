# Iconițe S4GGZ

## Cele 9 iconițe personalizabile din interfață

Adaugă fișierele în **`target\release\assets\`**, lângă imaginile existente. Acesta este folderul încărcat cu prioritate de aplicație. Nu le pune lângă executabilul de pornire din folderul principal.

| Nume exact | Unde apare | Sugestie de simbol |
| --- | --- | --- |
| `home.png` | Acasă | Casă |
| `songs.png` | Toate piesele / Explorează biblioteca | Notă muzicală |
| `albums.png` | Albume / Vezi toate albumele / Toate albumele | Disc sau grilă de albume |
| `favorites.png` | Secțiunea Favorite | Inimă |
| `add-music.png` | Adaugă muzică, importă/alege folder și Descarcă melodii în bara laterală | Folder cu plus |
| `rescan.png` | Rescanează / Actualizează biblioteca | Săgeată circulară |
| `settings.png` | Setări | Roată dințată |
| `queue.png` | Coada, inclusiv butonul din player | Listă de redare |
| `equalizer.png` | Egalizator | Trei slidere audio |

Recomandare: **PNG cu fundal transparent, 64 × 64 px**, pătrat, cu simbol lizibil la 18 × 18 px. Culorile originale se păstrează. Sunt acceptate și `.webp` / `.jpg` cu aceleași nume de bază; PNG are prioritate. Verifică să nu rezulte extensii duble precum `home.png.png`.

Închide și redeschide aplicația după adăugare. Nu este necesară recompilarea. Aceste iconițe sunt opționale: fără ele se afișează simbolurile integrate. Niciuna dintre cele 9 nu era prezentă ca fișier separat la pregătirea acestei liste.

## Iconița executabilului Windows

**Există deja:** `assets\aura-icon.ico`. Am încorporat-o în executabilul de pornire **`S4GGZ.exe`** din folderul principal. `assets\aura-icon.png` este varianta PNG existentă.

Pentru o eventuală înlocuire a iconiței EXE, folosește un `.ico` cu dimensiuni multiple, ideal 16, 32, 48 și 256 px. Înlocuirea fișierului `.ico` necesită reconstruirea executabilului de pornire pentru a actualiza iconița încorporată. Aceasta nu schimbă automat logo-ul desenat în interfața playerului sau iconița ferestrei lui.

## Imagini existente — nu trebuie adăugate din nou

În `target\release\assets\` sunt păstrate:

- `Aqua\bg.png` — fundalul Aqua;
- `Aqua\bubble.png` — textura bulelor;
- `Aqua\fish\nemo.png`, `Aqua\fish\fish1.png`, `Aqua\fish\fish2.png`, `Aqua\fish\fish3.png` — peștii animați;
- `Beer\beer-bg.png` — fundalul Beer, convertit în PNG din AVIF-ul furnizat;
- `Beer\beer.png` — halba folosită ca particulă (fișierul furnizat ca `beer-icon.png`);
- `Beer\beer-hand.png` — mâna animată cu bere;
- `discoball.png` — imaginea bilei pentru tema Disco; opțional, `Disco\discoball.png` o poate înlocui în aceeași locație de resurse;
- `aura-icon.ico`, `aura-icon.png` — grafica de branding existentă.

## Simboluri integrate care nu se înlocuiesc prin fișiere PNG

Noile butoane din Setări au iconițe vectoriale integrate: pește, pește tăiat, bulă (cerc cu un cerc mic descentrat în partea superioară), bulă tăiată, halbă, mână care ține o halbă, bilă disco, podea disco și imagine de fundal. Bila disco este reprezentată printr-o sferă cu oglinzi și fir de suspendare, iar podeaua printr-o grilă în perspectivă. Fiecare efect are și varianta tăiată pentru starea dezactivată. Toate aceste simboluri sunt monocrome. Pentru ele nu trebuie adăugat niciun fișier.

În tema **Disco**, iconițele bilei și podelei controlează efectele independent. Imaginea `discoball.png` este folosită pentru bila din scenă; iconița butonului din Setări este desenată în cod.

Play/Pause, Shuffle, Previous/Next, volumul, inimile din rândurile melodiilor, săgețile de paginare/reordonare, închiderea panourilor, simbolurile de copertă lipsă, logo-ul din bara laterală și săgeata butonului de descărcare din formular sunt desenate în cod sau sunt caractere text. Nu există în versiunea actuală fișiere externe precum `play.png`, `pause.png`, `shuffle.png`, `volume.png` sau `download.png` încărcate automat pentru acestea. Shuffle folosește săgeți vectoriale încrucișate și accentul temei când este activ. `favorites.png` schimbă iconița secțiunii Favorite; `add-music.png` este folosit și pentru navigarea la descărcări.

În Queue, X-ul de închidere și săgețile pentru prima poziție, sus, jos și ultima poziție sunt desenate vectorial, cu butoane compacte de 24 × 24 puncte; nu folosesc caractere de font. Coșul de gunoi din lista de melodii este desenat tot în cod și șterge fișierul audio, conform confirmării afișate. Butonul Egalizator înlocuiește Versuri; vechiul `lyrics.png` nu mai este utilizat în bara de sus.

Shuffle și Loop sunt desenate vectorial, inclusiv controlul oval mic suprapus în dreapta sus. Controlul mic selectează celălalt mod și schimbă pozițiile; cel mare arată modul activ. Sunt doar aceste două moduri exclusive, fără cifra 1 sau alte opțiuni intermediare. Nu sunt necesare fișiere `shuffle.png` sau `repeat.png`. Panoul Coada de descărcări folosește aceleași culori, sticlă și controale vectoriale ca tema activă.
