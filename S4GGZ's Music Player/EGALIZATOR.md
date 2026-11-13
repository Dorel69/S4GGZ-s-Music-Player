# Egalizator audio

Deschide **Egalizator** din colțul din dreapta al barei de sus. Cele 10 benzi și preamp-ul au slidere verticale, afișate alăturat într-o fereastră separată. Sus crește gain-ul, jos îl reduce. Închiderea ferestrei păstrează procesarea audio; închiderea playerului închide și fereastra de egalizare. **Egalizator activ** comută între reglajele curente și sunetul fără egalizare, printr-o tranziție de 30 ms.

Fereastra folosește fundalul temei active din player, inclusiv fotografia personalizată a acelei teme. Schimbarea temei sau a fotografiei se reflectă și în egalizator. Imaginea umple fereastra păstrând proporțiile și este desenată înaintea tuturor meniurilor, sliderelor și popup-urilor. Contrastul textului se adaptează separat în fiecare fereastră. Pentru tema Disco fără o fotografie, se folosește fundalul ambiental al temei.

Toate controalele sunt reunite într-un singur panou de **frosted glass**: titlul, presetul, importul/exportul, sliderele, preamp-ul și parametrii benzii selectate. În tema **Aqua**, doi pești animați înoată între fotografia de fundal și panoul de sticlă. Opțiunea **Pești** din Setări controlează și această animație. Egalizatorul are propriile poziții și propriul ceas de animație; peștii se opresc când fereastra este închisă sau tema nu este Aqua.

Cele 10 frecvențe inițiale sunt **60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000 și 16000 Hz**. Fiecare bandă are un slider de gain între **-12 și +12 dB**. Selectează o bandă apăsând frecvența de sub slider sau folosind sliderul, apoi reglează parametrii de sub slidere:

- frecvența centrală, între 20 și 20000 Hz;
- Q, între 0,2 și 10; valori mari concentrează efectul într-o zonă mai îngustă și pot produce rezonanță;
- Peaking, Low shelf sau High shelf. La shelf, frecvența definește mijlocul tranziției, iar Q controlează forma și rezonanța acesteia.

**Master gain / Preamp** reglează nivelul înaintea filtrelor, între **-36 și +12 dB**. Butonul de ajustare reduce conservator preamp-ul în funcție de amplificările benzilor. Preseturile includ această rezervă de nivel. Filtrele shelf cu Q mare pot produce vârfuri suplimentare; redu preamp-ul dacă sunetul devine comprimat. Protecția finală limitează vârfurile la nivel digital valid, cu atac imediat și revenire de 50 ms, folosind aceeași atenuare pentru toate canalele. Aceasta poate modifica dinamica la reglaje extreme; preamp-ul rămâne metoda preferată de a păstra headroom.

**Flat**, **Bass Boost**, **Treble Boost**, **Vocal** și **Rock** se aplică imediat, cu tranziție audio lină. Modificările manuale apar ca **Personalizat**. **Exportă JSON** salvează toate cele 10 benzi, preamp-ul și starea activă într-un fișier ales; **Importă JSON** le încarcă după validare. Fișierele invalide sau de peste 64 KiB sunt refuzate fără schimbarea sunetului. Setările curente se salvează automat în `%LOCALAPPDATA%\AuraPlayer\settings.json`, la 400 ms după ultima modificare și la închiderea normală a aplicației. Configurațiile vechi primesc implicit un egalizator Flat; un egalizator invalid este resetat separat, păstrând celelalte preferințe.

## Implementare

`src/equalizer.rs` conține structurile serializabile `Equalizer` și `FilterBand`, preseturile și adaptorul `EqualizerSource<S>`. Filtrele IIR de ordinul doi folosesc formulele RBJ din [W3C Audio EQ Cookbook](https://www.w3.org/TR/audio-eq-cookbook/), normalizate după a0. Procesarea folosește precizie f64 și Direct Form I, cu istorii independente pentru fiecare canal. La surse cu rată mică de eșantionare, frecvența efectivă este limitată la 45% din rata sursei pentru a rămâne sub Nyquist.

Traseul este **decoder → egalizator și protecție de vârf → analiză audio Disco → volumul playerului → ieșire**. Egalizatorul se aplică tuturor temelor și rămâne activ la trecerea la piesa următoare. Seek-ul resetează istoricul filtrelor, fără a pierde setările.

Interfața publică o configurație completă validată prin `Arc<Mutex<Equalizer>>`. Procesarea folosește exclusiv `try_lock` la fiecare 128 de cadre și păstrează configurația anterioară dacă interfața deține mutex-ul. Mișcările rapide ale sliderelor se combină în ultima configurație disponibilă, fără a umple coada comenzilor de redare. Coeficienții se recalculează numai la modificări sau schimbări de format și se interpolează liniar timp de 30 ms; gain-ul master urmează aceeași rampă. Interpolarea are loc o dată pe cadru audio, comun tuturor canalelor.

Starea, coeficienții și bufferul unui cadru au dimensiuni fixe, pentru maximum 32 de canale; egalizatorul nu alocă memorie, nu așteaptă mutex-uri și nu citește fișiere în bucla de procesare. Sursele cu peste 32 de canale sunt refuzate inițial. Pentru o sursă neobișnuită care își schimbă formatul la peste 32 de canale în timpul redării, adaptorul transmite eșantioanele cu limitare de nivel, fără egalizare. Valorile audio nefinite sunt înlocuite cu zero, iar stările foarte mici sunt eliminate pentru a evita denormalii.

`src/equalizer_ui.rs` gestionează interfața și operațiile JSON pe firul interfeței. Fereastra folosește un viewport egui deținut de player, cu minimum 760 × 500 puncte; nu pornește un proces separat. Ordinea de desenare este fotografie → pești Aqua → panou frosted glass → controale → popup-uri. Erorile de import, export și salvare sunt afișate în notificările aplicației.
